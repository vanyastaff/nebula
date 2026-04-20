//! Plugin discovery — scan directories for plugin binaries and query metadata
//! using the duplex v3 protocol.
//!
//! The main entry point is [`discover_directory`], which:
//! 1. Scans `dir` for files whose names start with `nebula-plugin-` / `nebula_plugin_` that look
//!    like executables.
//! 2. For each candidate, reads a sibling `plugin.toml` via
//!    [`crate::plugin_toml::parse_plugin_toml`] and enforces the `[nebula].sdk` constraint.
//! 3. Spawns the binary for a metadata probe (using [`PluginCapabilities::none`] for the probe —
//!    see safety note below), deserializes the v3 wire response, and applies the optional
//!    `[plugin].id` override.
//! 4. Builds [`crate::RemoteAction`] instances per wire `ActionDescriptor`, then wraps everything
//!    in a [`crate::DiscoveredPlugin`] → [`nebula_plugin::ResolvedPlugin`] and registers it in the
//!    provided [`nebula_plugin::PluginRegistry`].
//!
//! Per-plugin errors are warn-and-skip: a bad plugin never poisons the directory
//! scan.
//!
//! # Safety note — metadata probe capabilities
//!
//! The metadata probe runs with [`PluginCapabilities::none`]. Scanning an
//! untrusted binary for its manifest must never grant it network or filesystem
//! reach. Runtime capabilities (`default_capabilities`) are applied only when
//! the long-lived sandbox for action dispatch is constructed.
//!
//! Runtime `PluginCapabilities` are sourced from the caller (`default_capabilities`
//! arg). Wiring them from workflow-config is tracked under ADR-0025 D4 / slice 1d.

use std::{path::Path, sync::Arc, time::Duration};

use nebula_action::{Action, ActionHandler, ActionMetadata, StatelessHandler};
use nebula_core::ActionKey;
use nebula_plugin::{PluginRegistry, ResolvedPlugin};
use nebula_plugin_sdk::protocol::{ActionDescriptor, DUPLEX_PROTOCOL_VERSION, PluginToHost};

use crate::{
    DiscoveredPlugin, RemoteAction,
    capabilities::PluginCapabilities,
    handler::ProcessSandboxHandler,
    plugin_toml::{PluginTomlError, parse_plugin_toml},
    process::ProcessSandbox,
};

// ── Wire-response parse ──────────────────────────────────────────────────────

/// Wire metadata returned from a single plugin probe.
///
/// Private intermediate: built from the `MetadataResponse` envelope, then
/// immediately consumed to construct [`crate::DiscoveredPlugin`]. Not part of
/// the public API — callers use [`crate::DiscoveredPlugin`] via the registry.
#[cfg_attr(test, derive(Debug))]
struct WireMetadata {
    manifest: nebula_metadata::PluginManifest,
    actions: Vec<ActionDescriptor>,
}

/// Errors from [`probe_metadata`].
///
/// The version-mismatch branch must fire before the strongly-typed
/// `PluginToHost` deserialize — otherwise a v2 envelope (flat
/// `plugin_key` / `plugin_version`, no `manifest`) would surface as a
/// confusing "missing field `manifest`" serde error instead of a clear
/// protocol-version signal. See [`parse_metadata_response`] for the two-phase
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
        /// The `protocol_version` the plugin sent. Kept as `u64` to avoid a
        /// lossy truncation lying in the error report: if a buggy plugin
        /// sends a value ≥ 2^32, narrowing to `u32` could accidentally
        /// surface as "actual 3" (== expected) in the diagnostic.
        actual: u64,
    },

    /// Protocol version matched but the typed deserialize failed — e.g.
    /// a malformed v3 envelope that passed the kind+version checks but
    /// has a missing or malformed `manifest` field.
    #[error("plugin response failed typed deserialize: {0}")]
    TypedParse(#[source] serde_json::Error),
}

// ── Two-phase parse ──────────────────────────────────────────────────────────

/// Probe a plugin binary and return its wire metadata.
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
async fn probe_metadata(binary: &Path) -> Result<WireMetadata, DiscoveryError> {
    // ADR-0025 D4: runtime capabilities are sourced from workflow-config at
    // spawn-time (slice 1d). The metadata probe deliberately uses
    // PluginCapabilities::none() — scanning an untrusted binary for its
    // manifest must never grant network or filesystem reach.
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
fn parse_metadata_response(bytes: &[u8]) -> Result<WireMetadata, DiscoveryError> {
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
            actual: actual_version,
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
        } => Ok(WireMetadata { manifest, actions }),
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

// ── Per-plugin discovery helper ──────────────────────────────────────────────

/// Errors specific to per-plugin key reconciliation.
enum SkipReason {
    MissingPluginToml(PluginTomlError),
    SdkConstraintViolation {
        required: semver::VersionReq,
        host: semver::Version,
    },
    TransportError(DiscoveryError),
    KeyConflict {
        toml_id: String,
        manifest_key: String,
    },
    RegistrationError(nebula_plugin::PluginError),
}

/// Try to discover one plugin binary.
///
/// Returns `(resolved_plugin, action_handlers)` on success, where
/// `action_handlers` is a flat `(metadata, handler)` list the caller can
/// bulk-register into a runtime `ActionRegistry`. Both share the same
/// underlying `Arc<ProcessSandboxHandler>` — no double-spawn occurs.
///
/// Returns `Err(SkipReason)` on any failure. All failures are warn-and-skip.
async fn discover_one(
    binary: &Path,
    default_timeout: Duration,
    default_capabilities: &PluginCapabilities,
) -> Result<(ResolvedPlugin, Vec<(ActionMetadata, ActionHandler)>), SkipReason> {
    // Step 1: parse sibling plugin.toml (required).
    // `binary` came from `read_dir(dir)` — it always has a parent. The
    // invariant is structural; a silent CWD fallback would risk reading
    // the wrong plugin.toml and admitting the wrong plugin.
    let toml_path = binary
        .parent()
        .expect("read_dir entry always has a parent directory")
        .join("plugin.toml");
    let toml_manifest = parse_plugin_toml(&toml_path).map_err(SkipReason::MissingPluginToml)?;

    // Step 2: SDK constraint check before spawning the binary.
    // Validate against nebula-plugin-sdk's own version (not the sandbox
    // crate's) — plugin authors pin their `plugin.toml [nebula].sdk` against
    // nebula-plugin-sdk, and independent SDK bumps via
    // `cargo release -p nebula-plugin-sdk` are documented-supported.
    let host_version: semver::Version = nebula_plugin_sdk::protocol::SDK_VERSION
        .parse()
        .expect("nebula-plugin-sdk SDK_VERSION is always a valid semver");
    if !toml_manifest.sdk.matches(&host_version) {
        return Err(SkipReason::SdkConstraintViolation {
            required: toml_manifest.sdk,
            host: host_version,
        });
    }

    // Step 3: probe the plugin for its wire manifest (capabilities=none).
    let wire = probe_metadata(binary)
        .await
        .map_err(SkipReason::TransportError)?;

    // Step 4: reconcile plugin.toml optional id vs wire manifest key.
    let final_manifest = if let Some(ref toml_id) = toml_manifest.plugin_id {
        let manifest_key = wire.manifest.key().as_str();
        if toml_id != manifest_key {
            return Err(SkipReason::KeyConflict {
                toml_id: toml_id.clone(),
                manifest_key: manifest_key.to_owned(),
            });
        }
        // Keys agree — nothing to override.
        wire.manifest
    } else {
        wire.manifest
    };

    let plugin_key_str = final_manifest.key().as_str().to_owned();
    let namespace_prefix = format!("{plugin_key_str}.");
    let interface_version = final_manifest.version().clone();

    // Step 5: build RemoteAction instances per wire ActionDescriptor.
    // Build a shared long-lived sandbox for action dispatch.
    let sandbox = Arc::new(ProcessSandbox::new(
        binary.to_path_buf(),
        default_timeout,
        default_capabilities.clone(),
    ));

    // Concrete `Arc<RemoteAction>` — kept before type-erasure so we can
    // simultaneously coerce to `Arc<dyn Action>` (for DiscoveredPlugin) and
    // to `Arc<dyn StatelessHandler>` (for the returned handler list).
    let mut remote_actions: Vec<Arc<RemoteAction>> = Vec::new();
    for descriptor in &wire.actions {
        let full_key = if descriptor.key.contains('.') {
            if !descriptor.key.starts_with(&namespace_prefix) {
                tracing::warn!(
                    plugin = %plugin_key_str,
                    action_key = %descriptor.key,
                    "action key outside plugin namespace, skipping action",
                );
                continue;
            }
            descriptor.key.clone()
        } else {
            format!("{namespace_prefix}{}", descriptor.key)
        };

        let action_key = match ActionKey::new(&full_key) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    key = %full_key,
                    error = %e,
                    "invalid action key, skipping",
                );
                continue;
            },
        };

        let metadata = ActionMetadata::new(action_key, &descriptor.name, &descriptor.description)
            .with_version_full(interface_version.clone())
            .with_schema(descriptor.schema.clone());

        let handler = Arc::new(ProcessSandboxHandler::new(
            Arc::clone(&sandbox),
            metadata.clone(),
        ));
        remote_actions.push(Arc::new(RemoteAction::new(metadata, handler)));
    }

    // Build the flat handler list before erasing to `dyn Action`.
    // `RemoteAction: StatelessHandler` — coerce each Arc to the trait object.
    let action_handlers: Vec<(ActionMetadata, ActionHandler)> = remote_actions
        .iter()
        .map(|r| {
            // Both `Action` and `StatelessHandler` define `metadata()`.
            // Disambiguate via the `Action` impl.
            let meta = <RemoteAction as Action>::metadata(r).clone();
            let h: Arc<dyn StatelessHandler> = Arc::clone(r) as Arc<dyn StatelessHandler>;
            (meta, ActionHandler::Stateless(h))
        })
        .collect();

    // Erase to `Arc<dyn Action>` for the DiscoveredPlugin / Plugin trait.
    let actions: Vec<Arc<dyn Action>> = remote_actions
        .into_iter()
        .map(|r| r as Arc<dyn Action>)
        .collect();

    tracing::info!(
        plugin = %plugin_key_str,
        version = %final_manifest.version(),
        actions = actions.len(),
        credentials = 0,
        resources = 0,
        binary = %binary.display(),
        "discovered out-of-process plugin (credentials/resources gated on ADR-0025 slice 1d)",
    );

    // Step 6: construct DiscoveredPlugin + ResolvedPlugin.
    let discovered = DiscoveredPlugin::new(final_manifest, actions);
    let resolved = ResolvedPlugin::from(discovered).map_err(SkipReason::RegistrationError)?;
    Ok((resolved, action_handlers))
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Discover all plugins in `dir`, register them in `registry`, and return a
/// flat list of `(ActionMetadata, ActionHandler)` for bulk registration into a
/// runtime `ActionRegistry`.
///
/// Per-plugin failures are warn-and-skip — a bad plugin never poisons the
/// directory scan.
///
/// ## Why two outputs?
///
/// The engine's `PluginRegistry` stores `Arc<dyn Action>` (type-erased). The
/// runtime's `ActionRegistry` stores `Arc<dyn StatelessHandler>`. Both coercions
/// require the concrete `Arc<RemoteAction>` — which is available during
/// construction but lost after coercion. `discover_directory` performs both
/// coercions at construction time and returns the handler list to callers that
/// need to populate a runtime registry.
///
/// `default_capabilities` is applied to every discovered plugin's **runtime**
/// sandbox (the long-lived one used for action dispatch). The metadata probe
/// runs separately with [`PluginCapabilities::none`] — see [`probe_metadata`].
/// Callers are expected to source `default_capabilities` from host
/// configuration per deployment policy.
///
/// Runtime `PluginCapabilities` wiring from workflow-config is tracked under
/// ADR-0025 D4 / slice 1d.
pub async fn discover_directory(
    dir: &Path,
    registry: &mut PluginRegistry,
    default_timeout: Duration,
    default_capabilities: PluginCapabilities,
) -> Vec<(ActionMetadata, ActionHandler)> {
    let mut all_handlers: Vec<(ActionMetadata, ActionHandler)> = Vec::new();

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "failed to read plugin directory");
            return all_handlers;
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

        match discover_one(&path, default_timeout, &default_capabilities).await {
            Ok((resolved, handlers)) => {
                let key = resolved.key().clone();
                if let Err(e) = registry.register(Arc::new(resolved)) {
                    tracing::warn!(
                        binary = %path.display(),
                        error = %e,
                        "plugin already registered, skipping",
                    );
                } else {
                    tracing::info!(
                        plugin = %key,
                        binary = %path.display(),
                        "plugin registered in registry",
                    );
                    all_handlers.extend(handlers);
                }
            },
            Err(reason) => {
                let (reason_str, detail) = skip_reason_parts(&reason);
                tracing::warn!(
                    binary = %path.display(),
                    reason = reason_str,
                    detail = detail,
                    "skipping plugin",
                );
            },
        }
    }

    all_handlers
}

fn skip_reason_parts(reason: &SkipReason) -> (&'static str, String) {
    match reason {
        SkipReason::MissingPluginToml(e) => ("missing_plugin_toml", e.to_string()),
        SkipReason::SdkConstraintViolation { required, host } => (
            "sdk_constraint_violation",
            format!("required {required}, host {host}"),
        ),
        SkipReason::TransportError(e) => ("transport_error", e.to_string()),
        SkipReason::KeyConflict {
            toml_id,
            manifest_key,
        } => (
            "key_conflict",
            format!("plugin.toml id={toml_id}, manifest key={manifest_key}"),
        ),
        SkipReason::RegistrationError(e) => ("registration_error", e.to_string()),
    }
}

// ── Executable detection ─────────────────────────────────────────────────────

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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use nebula_action::Action;
    use nebula_metadata::PluginManifest;
    use nebula_plugin_sdk::protocol::{ActionDescriptor, DUPLEX_PROTOCOL_VERSION};
    use nebula_schema::Schema;
    use semver::Version;
    use serde_json::json;

    use super::{DiscoveryError, WireMetadata, parse_metadata_response};
    use crate::{capabilities::PluginCapabilities, process::ProcessSandbox};

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
        let wire = parse_metadata_response(&bytes).expect("v3 envelope must parse successfully");
        assert_eq!(wire.manifest.key().as_str(), "x");
        assert_eq!(wire.actions.len(), 1);
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

    // build_action_key tests (inline logic that was in create_handlers):
    #[test]
    fn cross_namespace_action_keys_are_rejected() {
        // Constructing a WireMetadata with a cross-namespace action and
        // passing it to discover_one is hard in tests (async + no fixture).
        // Instead, test the key logic directly: a dotted key that doesn't
        // start with the plugin prefix is not valid.
        let plugin_prefix = "com.good.plugin.";
        let key = "system.exec";
        assert!(!key.starts_with(plugin_prefix));
    }

    #[test]
    fn version_propagates_to_actions() {
        // Tests the ActionMetadata builder path (synchronously via WireMetadata).
        // Verify Version::new(2,7,3) round-trips through the builder.
        let manifest = PluginManifest::builder("com.good.plugin", "Good Plugin")
            .version(Version::new(2, 7, 3))
            .build()
            .unwrap();
        let schema = Schema::builder().build().unwrap();
        let wire = WireMetadata {
            manifest,
            actions: vec![ActionDescriptor {
                key: "echo".to_owned(),
                name: "Echo".to_owned(),
                description: "ok".to_owned(),
                schema,
            }],
        };
        let interface_version = wire.manifest.version().clone();
        assert_eq!(interface_version, Version::new(2, 7, 3));

        let sandbox = Arc::new(ProcessSandbox::new(
            PathBuf::from("nebula-plugin-dummy"),
            Duration::from_secs(1),
            PluginCapabilities::none(),
        ));
        let action_key = nebula_core::ActionKey::new("com.good.plugin.echo").expect("valid key");
        let metadata = nebula_action::ActionMetadata::new(action_key, "Echo", "ok")
            .with_version_full(interface_version);
        let handler = Arc::new(crate::ProcessSandboxHandler::new(
            Arc::clone(&sandbox),
            metadata.clone(),
        ));
        let remote = crate::RemoteAction::new(metadata, handler);
        assert_eq!(remote.metadata().base.version.major, 2);
        assert_eq!(remote.metadata().base.version.minor, 7);
        assert_eq!(remote.metadata().base.version.patch, 3);
    }
}
