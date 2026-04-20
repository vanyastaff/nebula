//! Plugin discovery — scan directories for plugin binaries and query metadata
//! using the duplex v3 protocol.

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
    /// Canonical plugin manifest.
    pub manifest: nebula_metadata::PluginManifest,
    /// Actions this plugin provides.
    pub actions: Vec<ActionDescriptor>,
}

/// Errors from [`discover_plugin`].
///
/// The version-mismatch branch must fire before the strongly-typed
/// `PluginToHost` deserialize — otherwise a v2 envelope (flat
/// `plugin_key` / `plugin_version`, no `manifest`) would surface as a
/// confusing "missing field `manifest`" serde error instead of a clear
/// protocol-version signal. See [`discover_plugin`] for the two-phase
/// parse that enforces this ordering.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    /// `ProcessSandbox` failed to spawn, dial, or round-trip the
    /// metadata request.
    #[error("discovery transport failed for plugin: {0}")]
    Transport(String),

    /// Response was not a `metadata_response` envelope.
    #[error("plugin returned unexpected envelope kind: {kind}")]
    UnexpectedEnvelope {
        /// The `kind` tag the plugin sent instead of `metadata_response`.
        kind: String,
    },

    /// Response envelope had no `kind` field or it wasn't a string.
    #[error("plugin response is missing a `kind` field")]
    MissingKind,

    /// Response envelope had no `protocol_version` field or it wasn't an integer.
    #[error("plugin response is missing a `protocol_version` field")]
    MissingProtocolVersion,

    /// Plugin speaks a different protocol version than the host.
    ///
    /// This variant is checked **before** the strongly-typed
    /// `PluginToHost` parse so that v2 plugins (which send
    /// `plugin_key` / `plugin_version` instead of `manifest`) get a clean
    /// version-mismatch error rather than a serde "missing field" message.
    #[error("protocol version mismatch: expected {expected}, actual {actual}")]
    ProtocolVersionMismatch {
        /// The `DUPLEX_PROTOCOL_VERSION` the host expects.
        expected: u32,
        /// The `protocol_version` the plugin sent.
        actual: u32,
    },

    /// Protocol version matched but the typed deserialize failed — e.g.
    /// a malformed v3 envelope that passed the kind+version checks but
    /// has a missing or malformed `manifest` field.
    #[error("plugin response failed typed deserialize: {0}")]
    TypedParse(#[source] serde_json::Error),
}

/// Discover a plugin by spawning its binary and sending a `MetadataRequest`
/// envelope.
///
/// The metadata probe is locked to [`PluginCapabilities::none`]: scanning an
/// untrusted binary for its metadata must never grant it network or filesystem
/// reach. Runtime capabilities are applied later, only when the host builds
/// the long-lived sandbox for action dispatch.
///
/// Uses a **two-phase parse**: the response bytes are first parsed to a
/// `serde_json::Value`, `kind` + `protocol_version` are checked, and only
/// then the *same bytes* are re-parsed via `serde_json::from_slice` into
/// `PluginToHost`. This ordering ensures a version-mismatched envelope
/// surfaces as [`DiscoveryError::ProtocolVersionMismatch`] rather than as
/// a confusing serde "missing field `manifest`" error that would fire
/// before the version branch under a one-shot typed parse. Re-parsing
/// from bytes (rather than from a `Value`) preserves the zero-copy /
/// borrowed-`&str` path that `domain_key::Key<T>::Deserialize` (and
/// therefore `PluginKey`) requires.
pub async fn discover_plugin(binary: &Path) -> Result<DiscoveredPlugin, DiscoveryError> {
    let sandbox = ProcessSandbox::new(
        binary.to_path_buf(),
        Duration::from_secs(5),
        PluginCapabilities::none(),
    );

    let bytes = sandbox
        .get_metadata_raw()
        .await
        .map_err(|e| DiscoveryError::Transport(format!("{}: {e}", binary.display())))?;

    parse_metadata_response(&bytes)
}

/// Two-phase parse of a raw metadata-response byte buffer.
///
/// Extracted so the unit tests can exercise the ordering invariant
/// (version check must fire before typed parse) without spinning up a
/// real plugin binary.
fn parse_metadata_response(bytes: &[u8]) -> Result<DiscoveredPlugin, DiscoveryError> {
    // Phase 1: untyped Value parse, just to inspect `kind` and
    // `protocol_version` before committing to the typed shape.
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(DiscoveryError::TypedParse)?;

    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or(DiscoveryError::MissingKind)?;
    if kind != "metadata_response" {
        return Err(DiscoveryError::UnexpectedEnvelope {
            kind: kind.to_owned(),
        });
    }

    let actual_version = value
        .get("protocol_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or(DiscoveryError::MissingProtocolVersion)?;
    if actual_version != u64::from(DUPLEX_PROTOCOL_VERSION) {
        return Err(DiscoveryError::ProtocolVersionMismatch {
            expected: DUPLEX_PROTOCOL_VERSION,
            actual: actual_version as u32,
        });
    }

    // Phase 2: strongly-typed parse. We parse from the original byte
    // slice (not from `value.clone()`) so that types relying on borrowed
    // `&str` deserialize — notably `PluginKey` via `domain_key::Key` —
    // keep working.
    let envelope: PluginToHost =
        serde_json::from_slice(bytes).map_err(DiscoveryError::TypedParse)?;

    match envelope {
        PluginToHost::MetadataResponse {
            manifest, actions, ..
        } => Ok(DiscoveredPlugin { manifest, actions }),
        other => Err(DiscoveryError::UnexpectedEnvelope {
            kind: response_kind(&other).to_owned(),
        }),
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
/// `default_capabilities` is applied to every discovered plugin's **runtime**
/// sandbox (the long-lived one used for action dispatch). The metadata probe
/// runs separately with [`PluginCapabilities::none`] — see [`discover_plugin`].
/// Callers are expected to source `default_capabilities` from host
/// configuration per deployment policy.
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

        match discover_plugin(&path).await {
            Ok(plugin) => {
                let sandbox = Arc::new(ProcessSandbox::new(
                    path.clone(),
                    default_timeout,
                    default_capabilities.clone(),
                ));
                let handlers = create_handlers(&plugin, sandbox);

                tracing::info!(
                    plugin = %plugin.manifest.key(),
                    version = %plugin.manifest.version(),
                    actions = handlers.len(),
                    binary = %path.display(),
                    "discovered community plugin"
                );

                results.push((plugin.manifest.key().as_str().to_owned(), handlers));
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
    let plugin_key = plugin.manifest.key().as_str();
    let namespace_prefix = format!("{plugin_key}.");
    let interface_version = plugin.manifest.version().clone();
    plugin
        .actions
        .iter()
        .filter_map(|action| {
            let full_key = if action.key.contains('.') {
                if !action.key.starts_with(&namespace_prefix) {
                    tracing::warn!(
                        plugin = %plugin_key,
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

    use nebula_metadata::PluginManifest;
    use nebula_plugin_sdk::protocol::{ActionDescriptor, DUPLEX_PROTOCOL_VERSION};
    use nebula_schema::Schema;
    use semver::Version;
    use serde_json::json;

    use super::{DiscoveredPlugin, DiscoveryError, create_handlers, parse_metadata_response};
    use crate::{capabilities::PluginCapabilities, process::ProcessSandbox};

    fn empty_schema() -> nebula_schema::ValidSchema {
        Schema::builder().build().unwrap()
    }

    #[test]
    fn v2_envelope_surfaces_as_protocol_version_mismatch_not_missing_field() {
        // Simulates a v2 plugin replying with the old flat
        // `plugin_key` / `plugin_version` shape (no `manifest`, no
        // per-action `schema`). A one-shot typed parse would fail with
        // "missing field `manifest`" before the version check; the
        // two-phase parse must surface the version mismatch instead.
        let v2_envelope = json!({
            "kind": "metadata_response",
            "id": 1,
            "protocol_version": 2,
            "plugin_key": "x",
            "plugin_version": "1.0.0",
            "actions": [],
        });
        let bytes = serde_json::to_vec(&v2_envelope).unwrap();

        let err =
            parse_metadata_response(&bytes).expect_err("v2 envelope must not deserialize as v3");

        match err {
            DiscoveryError::ProtocolVersionMismatch { expected, actual } => {
                assert_eq!(expected, DUPLEX_PROTOCOL_VERSION);
                assert_eq!(expected, 3);
                assert_eq!(actual, 2);
            },
            other => panic!(
                "expected DiscoveryError::ProtocolVersionMismatch, got {other:?} \
                 (likely a serde missing-field error, which means the version \
                 check is firing too late)"
            ),
        }
    }

    #[test]
    fn v3_envelope_with_manifest_round_trips_through_two_phase_parse() {
        // Sanity: a well-formed v3 envelope still deserializes cleanly
        // through the two-phase path.
        let manifest = PluginManifest::builder("x", "X").build().unwrap();
        let schema = Schema::builder().build().unwrap();
        let envelope = nebula_plugin_sdk::protocol::PluginToHost::MetadataResponse {
            id: 1,
            protocol_version: DUPLEX_PROTOCOL_VERSION,
            manifest,
            actions: vec![ActionDescriptor {
                key: "echo".into(),
                name: "Echo".into(),
                description: String::new(),
                schema,
            }],
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let discovered =
            parse_metadata_response(&bytes).expect("v3 envelope must parse successfully");
        assert_eq!(discovered.manifest.key().as_str(), "x");
        assert_eq!(discovered.actions.len(), 1);
    }

    #[test]
    fn unexpected_envelope_kind_surfaces_cleanly() {
        let value = json!({
            "kind": "action_result_ok",
            "id": 1,
            "output": {},
        });
        let bytes = serde_json::to_vec(&value).unwrap();
        let err = parse_metadata_response(&bytes).expect_err("non-metadata envelope must error");
        match err {
            DiscoveryError::UnexpectedEnvelope { kind } => {
                assert_eq!(kind, "action_result_ok");
            },
            other => panic!("expected UnexpectedEnvelope, got {other:?}"),
        }
    }

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
        let manifest = PluginManifest::builder("com.good.plugin", "Good Plugin")
            .build()
            .unwrap();
        let plugin = DiscoveredPlugin {
            manifest,
            actions: vec![
                ActionDescriptor {
                    key: "echo".to_owned(),
                    name: "Echo".to_owned(),
                    description: "ok".to_owned(),
                    schema: empty_schema(),
                },
                ActionDescriptor {
                    key: "system.exec".to_owned(),
                    name: "Exec".to_owned(),
                    description: "bad".to_owned(),
                    schema: empty_schema(),
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
        let manifest = PluginManifest::builder("com.good.plugin", "Good Plugin")
            .version(Version::new(2, 7, 3))
            .build()
            .unwrap();
        let plugin = DiscoveredPlugin {
            manifest,
            actions: vec![ActionDescriptor {
                key: "echo".to_owned(),
                name: "Echo".to_owned(),
                description: "ok".to_owned(),
                schema: empty_schema(),
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
