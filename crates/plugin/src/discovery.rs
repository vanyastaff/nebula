//! Plugin discovery — scan directories for plugin binaries and query metadata
//! using the duplex v3 protocol.
//!
//! The main entry point is [`discover_directory`], which:
//! 1. Scans `dir` for files whose names start with `nebula-plugin-` / `nebula_plugin_` that look
//!    like executables.
//! 2. For each candidate, reads a sibling `plugin.toml` via
//!    [`crate::plugin_toml::parse_plugin_toml`] and enforces the `[nebula].sdk` constraint.
//! 3. Spawns the binary for a metadata probe, deserializes the v3 wire response, and applies the
//!    optional `[plugin].id` override.
//! 4. Builds [`crate::RemoteAction`] instances per wire `ActionDescriptor`, then wraps everything
//!    in a [`crate::DiscoveredPlugin`] → [`crate::ResolvedPlugin`] and registers it in the
//!    provided [`crate::PluginRegistry`].
//!
//! Per-plugin errors are warn-and-skip: a bad plugin never poisons the directory
//! scan.
//!
//! There is no per-plugin capability/scope model in this path: egress,
//! credential, and filesystem mediation is the broker's responsibility
//!, not discovery. The probe and the runtime
//! sandbox spawn the binary with the same OS-level hardening.

use std::{path::Path, sync::Arc, time::Duration};

use nebula_action::{ActionFactory, ActionHandler, ActionMetadata, StatelessHandler};
use nebula_core::ActionKey;
use nebula_plugin_sdk::protocol::{ActionDescriptor, DUPLEX_PROTOCOL_VERSION, PluginToHost};
use nebula_sandbox::ProcessSandbox;

use crate::{
    DiscoveredPlugin, PluginRegistry, RemoteAction, ResolvedPlugin,
    handler::ProcessSandboxHandler,
    plugin_toml::{PluginTomlError, parse_plugin_toml},
    remote_action::RemoteActionFactory,
};

// ── Discovered-action record ─────────────────────────────────────────────────

/// One discovered out-of-process action: its host-side metadata, the
/// dispatch handler built during discovery, and the plugin binary it came
/// from.
///
/// The `binary` is exposed so an engine-side composition root can key an
/// engine-owned plugin-process pool on `(binary, scope)`.
/// The pre-existing `handler` (a `ProcessSandboxHandler` over one
/// long-lived process) is unchanged — the binary is purely additive
/// context for callers that pool processes themselves.
#[derive(Clone)]
pub struct DiscoveredAction {
    /// Plugin binary this action is dispatched to.
    pub binary: std::path::PathBuf,
    /// Host-side metadata resolved from the wire `ActionDescriptor`. Its
    /// `base.key` is **namespaced** (`<plugin>.<local>`).
    pub metadata: ActionMetadata,
    /// The raw wire `ActionDescriptor.key` the plugin matches on in its
    /// own `PluginHandler::execute` — i.e. the un-namespaced local key. A
    /// pooling caller MUST send this (not the namespaced metadata key)
    /// over the transport, otherwise the plugin rejects the invocation
    /// with `UNKNOWN_ACTION`.
    pub local_key: String,
    /// Discovery-built dispatch handler (shared long-lived process).
    pub handler: ActionHandler,
}

impl std::fmt::Debug for DiscoveredAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveredAction")
            .field("binary", &self.binary)
            .field("key", &self.metadata.base.key)
            .field("local_key", &self.local_key)
            .finish_non_exhaustive()
    }
}

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

/// Errors from `probe_metadata`.
///
/// The version-mismatch branch must fire before the strongly-typed
/// `PluginToHost` deserialize — otherwise a v2 envelope (flat
/// `plugin_key` / `plugin_version`, no `manifest`) would surface as a
/// confusing "missing field `manifest`" serde error instead of a clear
/// protocol-version signal. The private `parse_metadata_response` helper
/// implements the two-phase parse that enforces this ordering.
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
/// The probe spawns the binary with the same OS-level hardening as the
/// runtime sandbox. There is no per-plugin capability grant (;
/// scope model is the broker's ).
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
    let sandbox = ProcessSandbox::new(binary.to_path_buf(), Duration::from_secs(5));

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
    /// `binary` unexpectedly has no parent directory (impossible when yielded
    /// by `read_dir(dir)`, but we route through `SkipReason` rather than
    /// panic so library code never crashes on a malformed path).
    NoBinaryParent {
        binary: std::path::PathBuf,
    },
    /// `nebula-plugin-sdk`'s `SDK_VERSION` constant is not a valid semver.
    /// This is a compile-time invariant of the SDK crate and should never
    /// fire in a shipped binary, but routing through `SkipReason` keeps
    /// discovery panic-free even in pathological build states.
    HostSdkVersionInvalid {
        raw: &'static str,
        source: semver::Error,
    },
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
    /// A wire `ActionDescriptor` carries a fully-qualified key that belongs to
    /// a *different* plugin's namespace. This is a whole-plugin failure (not a
    /// per-action skip) because a plugin that lies about its action namespace
    /// cannot be trusted — fail fast and surface the violation at load time.
    /// Symmetric with [`crate::ResolvedPlugin::from`]'s fail-fast
    /// behaviour for in-process plugins.
    CrossNamespaceAction {
        descriptor_key: String,
        plugin_key: String,
    },
    RegistrationError(crate::PluginError),
}

/// Sentinel returned by [`resolve_action_key`] when the descriptor key is
/// fully-qualified but belongs to a *different* plugin's namespace.
///
/// The caller decides how to surface this — currently: fail the whole plugin
/// via [`SkipReason::CrossNamespaceAction`].
#[derive(Debug)]
struct CrossNamespace;

/// Resolve a wire action descriptor key against a plugin's namespace prefix.
///
/// Returns `Ok(Some(full_key))` when the descriptor key is either the short
/// local form (no dot — automatically prefixed) or a fully-qualified key that
/// falls inside the plugin's own namespace.
///
/// Returns `Ok(None)` when the resulting full key is syntactically invalid
/// (e.g. empty local part that produces a key `ActionKey::new` rejects).
/// Callers treat this as a per-action warn-and-skip (different from the
/// cross-namespace case, which fails the whole plugin).
///
/// Returns `Err(CrossNamespace)` when the descriptor key is fully-qualified
/// but belongs to a *different* plugin's namespace. The caller should fail
/// the whole plugin discovery, not just skip the action.
///
/// # Arguments
///
/// * `namespace_prefix` — e.g. `"com.author.slack."` (must end with `.`).
/// * `descriptor_key` — the raw key string from the wire `ActionDescriptor`.
fn resolve_action_key(
    namespace_prefix: &str,
    descriptor_key: &str,
) -> Result<Option<ActionKey>, CrossNamespace> {
    let full_key = if descriptor_key.contains('.') {
        // Fully-qualified: must start with our own namespace prefix.
        if !descriptor_key.starts_with(namespace_prefix) {
            return Err(CrossNamespace);
        }
        descriptor_key.to_owned()
    } else {
        // Short local form: prepend namespace.
        format!("{namespace_prefix}{descriptor_key}")
    };
    Ok(ActionKey::new(&full_key).ok())
}

/// Try to discover one plugin binary.
///
/// Returns `(resolved_plugin, discovered_actions)` on success, where
/// `discovered_actions` is a flat [`DiscoveredAction`] list (metadata +
/// handler + binary) the caller can bulk-register into a runtime
/// `ActionRegistry`. All actions share the same underlying
/// `Arc<ProcessSandboxHandler>` — no double-spawn occurs.
///
/// Returns `Err(SkipReason)` on any failure. All failures are warn-and-skip.
async fn discover_one(
    binary: &Path,
    default_timeout: Duration,
) -> Result<(ResolvedPlugin, Vec<DiscoveredAction>), SkipReason> {
    // Step 1: parse sibling plugin.toml (required).
    // `binary` came from `read_dir(dir)` — it always has a parent in practice.
    // Route the theoretically-impossible `None` case through `SkipReason`
    // rather than panic, and never fall back to CWD (a silent CWD lookup
    // would read the wrong plugin.toml and admit the wrong plugin).
    let toml_path = binary
        .parent()
        .ok_or_else(|| SkipReason::NoBinaryParent {
            binary: binary.to_path_buf(),
        })?
        .join("plugin.toml");
    let toml_manifest = parse_plugin_toml(&toml_path).map_err(SkipReason::MissingPluginToml)?;

    // Step 2: SDK constraint check before spawning the binary.
    // Validate against nebula-plugin-sdk's own version (not the sandbox
    // crate's) — plugin authors pin their `plugin.toml [nebula].sdk` against
    // nebula-plugin-sdk, and independent SDK bumps via
    // `cargo release -p nebula-plugin-sdk` are documented-supported.
    let host_version: semver::Version =
        nebula_plugin_sdk::protocol::SDK_VERSION
            .parse()
            .map_err(|source| SkipReason::HostSdkVersionInvalid {
                raw: nebula_plugin_sdk::protocol::SDK_VERSION,
                source,
            })?;
    if !toml_manifest.sdk.matches(&host_version) {
        return Err(SkipReason::SdkConstraintViolation {
            required: toml_manifest.sdk,
            host: host_version,
        });
    }

    // Step 3: probe the plugin for its wire manifest.
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
    let sandbox = Arc::new(ProcessSandbox::new(binary.to_path_buf(), default_timeout));

    // Concrete `Arc<RemoteAction>` — kept before type-erasure so we can
    // simultaneously wrap as `Arc<dyn ActionFactory>` (for DiscoveredPlugin via
    // RemoteActionFactory, post-Variant A) and coerce to `Arc<dyn StatelessHandler>`
    // (for the returned handler list).
    let mut remote_actions: Vec<(Arc<RemoteAction>, String)> = Vec::new();
    for descriptor in &wire.actions {
        let action_key = match resolve_action_key(&namespace_prefix, &descriptor.key) {
            // Cross-namespace: fail the whole plugin (symmetric with
            // ResolvedPlugin::from's behaviour for in-process plugins).
            Err(CrossNamespace) => {
                return Err(SkipReason::CrossNamespaceAction {
                    descriptor_key: descriptor.key.clone(),
                    plugin_key: plugin_key_str,
                });
            },
            // Invalid syntax (e.g. empty local part): warn and skip this
            // individual action only — different from cross-namespace.
            Ok(None) => {
                tracing::warn!(
                    plugin = %plugin_key_str,
                    descriptor_key = %descriptor.key,
                    "invalid action key syntax, skipping action",
                );
                continue;
            },
            Ok(Some(k)) => k,
        };

        let metadata = ActionMetadata::new(action_key, &descriptor.name, &descriptor.description)
            .with_version_full(interface_version.clone())
            .with_schema(descriptor.schema.clone());

        // The plugin matches on the un-namespaced wire `descriptor.key` in
        // its own `PluginHandler::execute`; the handler must send that, not
        // the namespaced `metadata.base.key`.
        let handler = Arc::new(ProcessSandboxHandler::new(
            Arc::clone(&sandbox),
            metadata.clone(),
            descriptor.key.clone(),
        ));
        remote_actions.push((
            Arc::new(RemoteAction::new(metadata, handler)),
            descriptor.key.clone(),
        ));
    }

    // Build the flat discovered-action list — coerce each
    // Arc<RemoteAction> to dyn StatelessHandler and carry the binary +
    // plugin-local key so a pooling caller can key on (binary, scope) and
    // send the key the plugin actually matches on.
    let action_handlers: Vec<DiscoveredAction> = remote_actions
        .iter()
        .map(|(r, local_key)| {
            let metadata = r.metadata().clone();
            let h: Arc<dyn StatelessHandler> = Arc::clone(r) as Arc<dyn StatelessHandler>;
            DiscoveredAction {
                binary: binary.to_path_buf(),
                metadata,
                local_key: local_key.clone(),
                handler: ActionHandler::Stateless(h),
            }
        })
        .collect();

    // Wrap each in `RemoteActionFactory` for `DiscoveredPlugin` / `Plugin` trait.
    let actions: Vec<Arc<dyn ActionFactory>> = remote_actions
        .into_iter()
        .map(|(r, _local_key)| {
            let factory = RemoteActionFactory::new(r);
            Arc::new(factory) as Arc<dyn ActionFactory>
        })
        .collect();

    tracing::info!(
        plugin = %plugin_key_str,
        version = %final_manifest.version(),
        actions = actions.len(),
        credentials = 0,
        resources = 0,
        binary = %binary.display(),
        "discovered out-of-process plugin (credentials/resources gated on broker slice 1d)",
    );

    // Step 6: construct DiscoveredPlugin + ResolvedPlugin.
    let discovered = DiscoveredPlugin::new(final_manifest, actions);
    let resolved = ResolvedPlugin::from(discovered).map_err(SkipReason::RegistrationError)?;
    Ok((resolved, action_handlers))
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Discover all plugins in `dir`, register them in `registry`, and return a
/// flat `Vec<DiscoveredAction>` (each [`DiscoveredAction`] carries metadata +
/// handler + binary + plugin-local key) for bulk registration into a runtime
/// `ActionRegistry`.
///
/// Per-plugin failures are warn-and-skip — a bad plugin never poisons the
/// directory scan.
///
/// ## Why two outputs?
///
/// The engine's `PluginRegistry` stores `Arc<dyn ActionFactory>` per action key.
/// The runtime's `ActionRegistry` stores
/// `Arc<dyn StatelessHandler>`. Both wrappings require the concrete
/// `Arc<RemoteAction>` — which is available during construction but lost
/// after coercion. `discover_directory` performs both at construction time
/// and returns the [`DiscoveredAction`] list to callers that need to
/// populate a runtime registry. Each record also carries the plugin
/// `binary` so an engine-side pooling caller can key on `(binary, scope)`.
///
/// Per-plugin capability/scope is **not** modeled here — egress, credential,
/// and filesystem mediation is the broker's responsibility, not
/// this discovery path.
pub async fn discover_directory(
    dir: &Path,
    registry: &mut PluginRegistry,
    default_timeout: Duration,
) -> Vec<DiscoveredAction> {
    let mut all_handlers: Vec<DiscoveredAction> = Vec::new();

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

        match discover_one(&path, default_timeout).await {
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
        SkipReason::NoBinaryParent { binary } => (
            "no_binary_parent",
            format!("binary path has no parent directory: {}", binary.display()),
        ),
        SkipReason::HostSdkVersionInvalid { raw, source } => (
            "host_sdk_version_invalid",
            format!("nebula-plugin-sdk SDK_VERSION {raw:?} is not valid semver: {source}"),
        ),
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
        SkipReason::CrossNamespaceAction {
            descriptor_key,
            plugin_key,
        } => (
            "cross_namespace_action",
            format!("action key {descriptor_key:?} is outside plugin namespace {plugin_key:?}"),
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

    use nebula_metadata::PluginManifest;
    use nebula_plugin_sdk::protocol::{ActionDescriptor, DUPLEX_PROTOCOL_VERSION};
    use semver::Version;
    use serde_json::json;

    use nebula_sandbox::ProcessSandbox;

    use super::{DiscoveryError, WireMetadata, parse_metadata_response};

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
        let schema = nebula_schema::ValidSchema::empty();
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

    // Unit tests for the extracted resolve_action_key helper.
    use super::resolve_action_key;

    #[test]
    fn resolve_action_key_accepts_short_local() {
        let full = resolve_action_key("com.good.plugin.", "echo")
            .unwrap()
            .unwrap();
        assert_eq!(full.as_str(), "com.good.plugin.echo");
    }

    #[test]
    fn resolve_action_key_accepts_fully_qualified_within_namespace() {
        let full = resolve_action_key("com.good.plugin.", "com.good.plugin.echo")
            .unwrap()
            .unwrap();
        assert_eq!(full.as_str(), "com.good.plugin.echo");
    }

    #[test]
    fn resolve_action_key_rejects_cross_namespace() {
        // A dotted key that does not start with the plugin's own prefix
        // must return Err(CrossNamespace), not Ok.
        let result = resolve_action_key("com.good.plugin.", "system.exec");
        assert!(
            result.is_err(),
            "cross-namespace key must return Err(CrossNamespace)"
        );
    }

    #[test]
    fn resolve_action_key_returns_none_for_invalid_syntax() {
        // An empty local part produces a key string that ActionKey::new rejects.
        let result = resolve_action_key("com.good.plugin.", "").unwrap();
        assert!(
            result.is_none(),
            "empty local part must produce Ok(None), not a valid key"
        );
    }

    #[test]
    fn version_propagates_to_actions() {
        // Tests the ActionMetadata builder path (synchronously via WireMetadata).
        // Verify Version::new(2,7,3) round-trips through the builder.
        let manifest = PluginManifest::builder("com.good.plugin", "Good Plugin")
            .version(Version::new(2, 7, 3))
            .build()
            .unwrap();
        let schema = nebula_schema::ValidSchema::empty();
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
        ));
        let action_key = nebula_core::ActionKey::new("com.good.plugin.echo").expect("valid key");
        let metadata = nebula_action::ActionMetadata::new(action_key, "Echo", "ok")
            .with_version_full(interface_version);
        let handler = Arc::new(crate::ProcessSandboxHandler::new(
            Arc::clone(&sandbox),
            metadata.clone(),
            "echo".to_owned(),
        ));
        let remote = crate::RemoteAction::new(metadata, handler);
        assert_eq!(remote.metadata().base.version.major, 2);
        assert_eq!(remote.metadata().base.version.minor, 7);
        assert_eq!(remote.metadata().base.version.patch, 3);
    }
}
