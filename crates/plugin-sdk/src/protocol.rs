//! Wire protocol: tagged envelope types for the duplex broker stream.
//!
//! This module defines the on-the-wire shapes used by both the plugin and
//! the host. Plugin authors never touch these types directly — they work
//! with [`PluginHandler`](crate::PluginHandler) / [`PluginCtx`](crate::PluginCtx)
//! in the parent module. The host (`nebula-sandbox`) imports these types to
//! (de)serialize envelopes over the transport.
//!
//! ## Framing
//!
//! - Line-delimited JSON. One message per `\n`.
//! - Serialized envelopes must never contain raw newlines — `serde_json::to_string` escapes them
//!   inside string values, so this is automatic. The [`single_line_serialization`](#testing) test
//!   locks the invariant.
//! - Readers split on `\n`, trim whitespace, parse each non-empty line.
//! - Writers `write_all(encoded)` then `write_all(b"\n")` then `flush`.
//!
//! ## Transport
//!
//! - Slices 1a / 1b: stdio (parent pipes to child's stdin/stdout).
//! - Slice 1c: Unix domain socket (Linux/macOS) or Named Pipe (Windows), with the parent dialing an
//!   address the plugin announces via a one-line handshake printed to stdout before the listener
//!   accepts.
//!
//! The envelope shape is transport-agnostic — the same types flow over any
//! byte stream that implements `AsyncRead + AsyncWrite`.

use serde::{Deserialize, Serialize};

/// Duplex protocol version. Bumped when wire format changes incompatibly.
pub const DUPLEX_PROTOCOL_VERSION: u32 = 3;

/// SDK version plugin authors compile against. Host reads this to
/// validate the plugin's `plugin.toml [nebula].sdk` semver constraint.
/// The `env!` expansion happens at SDK crate compile, so it reflects the
/// SDK's own version — not whatever crate consumes this constant.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Message from host to plugin.
///
/// Tagged by `kind`. Correlation IDs (`id`) are host-assigned `u64` values that
/// the plugin echoes back in the corresponding response message. IDs are
/// unique within a single plugin process lifetime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostToPlugin {
    /// Invoke an action.
    ActionInvoke {
        /// Correlation id, echoed back in the matching `ActionResultOk`/`ActionResultError`.
        id: u64,
        /// Action key to invoke (e.g., `"com.author.telegram.send_message"`).
        action_key: String,
        /// Input payload for the action.
        input: serde_json::Value,
    },
    /// Cooperative cancel signal for an in-flight action.
    Cancel {
        /// Correlation id of the action to cancel.
        id: u64,
    },
    /// Successful response to a plugin-initiated [`PluginToHost::RpcCall`].
    RpcResponseOk {
        /// Correlation id matching the original `RpcCall.id`.
        id: u64,
        /// Result payload.
        result: serde_json::Value,
    },
    /// Error response to a plugin-initiated [`PluginToHost::RpcCall`].
    RpcResponseError {
        /// Correlation id matching the original `RpcCall.id`.
        id: u64,
        /// Machine-readable error code.
        code: String,
        /// Human-readable error message.
        message: String,
    },
    /// Request plugin metadata. Plugin must respond with [`PluginToHost::MetadataResponse`].
    MetadataRequest {
        /// Correlation id.
        id: u64,
    },
    /// Graceful shutdown. Plugin should flush any pending output and exit.
    Shutdown,
}

/// Message from plugin to host.
///
/// Tagged by `kind`. Plugins send these as responses to host requests
/// (`ActionResult*`, `MetadataResponse`) or as plugin-initiated events
/// (`RpcCall`, `Log`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "MetadataResponse carries a PluginManifest (~500 bytes) and is \
              built exactly once per plugin handshake; the other variants \
              are the hot path and are already small. Boxing the manifest \
              would add a heap alloc + indirection for a one-shot \
              construction and no runtime gain."
)]
pub enum PluginToHost {
    /// Successful action result.
    ActionResultOk {
        /// Correlation id from the original `ActionInvoke`.
        id: u64,
        /// Action output payload.
        output: serde_json::Value,
    },
    /// Failed action result.
    ActionResultError {
        /// Correlation id from the original `ActionInvoke`.
        id: u64,
        /// Machine-readable error code (e.g., `"VALIDATION"`, `"TIMEOUT"`).
        code: String,
        /// Human-readable error message.
        message: String,
        /// Whether the error is transient and the host may retry.
        #[serde(default)]
        retryable: bool,
    },
    /// Plugin-initiated RPC call into the host broker.
    ///
    /// Host must respond with [`HostToPlugin::RpcResponseOk`] or
    /// [`HostToPlugin::RpcResponseError`] matching `id`.
    RpcCall {
        /// Plugin-assigned correlation id.
        id: u64,
        /// RPC verb (e.g., `"credentials.get"`, `"network.http_request"`).
        verb: String,
        /// Verb-specific parameters.
        params: serde_json::Value,
    },
    /// One-way structured log entry. No response expected.
    Log {
        /// Log level.
        level: LogLevel,
        /// Log message.
        message: String,
        /// Structured fields (free-form JSON object).
        #[serde(default)]
        fields: serde_json::Value,
    },
    /// Plugin metadata response (reply to [`HostToPlugin::MetadataRequest`]).
    MetadataResponse {
        /// Correlation id from the original `MetadataRequest`.
        id: u64,
        /// Protocol version the plugin speaks. Host verifies compatibility.
        protocol_version: u32,
        /// Canonical bundle descriptor (slice B replaced the flat
        /// `plugin_key` / `plugin_version` fields with the full manifest).
        manifest: nebula_metadata::PluginManifest,
        /// Actions this plugin provides.
        actions: Vec<ActionDescriptor>,
    },
}

/// Structured log level (subset of `tracing::Level`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Trace level — very verbose.
    Trace,
    /// Debug level.
    Debug,
    /// Info level.
    Info,
    /// Warn level.
    Warn,
    /// Error level.
    Error,
}

/// Describes one action offered by a plugin. Wire DTO — maps onto
/// `nebula-action::ActionMetadata` once discovery converts it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionDescriptor {
    /// Action key — short local form (`"send_message"`) or already
    /// namespace-qualified (`"slack.send_message"`). Host validates.
    pub key: String,
    /// Human-readable action name.
    pub name: String,
    /// Optional human-readable description.
    #[serde(default)]
    pub description: String,
    /// Input schema the host uses to validate user-supplied parameters.
    pub schema: nebula_schema::ValidSchema,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn host_to_plugin_action_invoke_roundtrip() {
        let msg = HostToPlugin::ActionInvoke {
            id: 42,
            action_key: "telegram.send".into(),
            input: json!({"chat_id": 123, "text": "hi"}),
        };
        let line = serde_json::to_string(&msg).unwrap();
        assert!(line.contains(r#""kind":"action_invoke""#));
        assert!(line.contains(r#""id":42"#));
        let parsed: HostToPlugin = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn plugin_to_host_action_result_roundtrip() {
        let msg = PluginToHost::ActionResultOk {
            id: 42,
            output: json!({"message_id": 999}),
        };
        let line = serde_json::to_string(&msg).unwrap();
        assert!(line.contains(r#""kind":"action_result_ok""#));
        let parsed: PluginToHost = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn plugin_to_host_action_error_roundtrip() {
        let msg = PluginToHost::ActionResultError {
            id: 42,
            code: "TIMEOUT".into(),
            message: "took too long".into(),
            retryable: true,
        };
        let line = serde_json::to_string(&msg).unwrap();
        let parsed: PluginToHost = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn metadata_request_and_response_roundtrip() {
        let req = HostToPlugin::MetadataRequest { id: 1 };
        let line = serde_json::to_string(&req).unwrap();
        assert!(line.contains(r#""kind":"metadata_request""#));

        let manifest = nebula_metadata::PluginManifest::builder("com.author.echo", "Echo")
            .version(semver::Version::new(1, 0, 0))
            .build()
            .unwrap();

        let schema = nebula_schema::Schema::builder().build().unwrap();

        let resp = PluginToHost::MetadataResponse {
            id: 1,
            protocol_version: DUPLEX_PROTOCOL_VERSION,
            manifest,
            actions: vec![ActionDescriptor {
                key: "echo".into(),
                name: "Echo".into(),
                description: "Echoes input".into(),
                schema,
            }],
        };
        let line = serde_json::to_string(&resp).unwrap();
        let parsed: PluginToHost = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn rpc_call_roundtrip() {
        let msg = PluginToHost::RpcCall {
            id: 7,
            verb: "credentials.get".into(),
            params: json!({"slot": "bot_token"}),
        };
        let line = serde_json::to_string(&msg).unwrap();
        assert!(line.contains(r#""kind":"rpc_call""#));
        let parsed: PluginToHost = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn log_roundtrip() {
        let msg = PluginToHost::Log {
            level: LogLevel::Info,
            message: "action started".into(),
            fields: json!({"action": "echo"}),
        };
        let line = serde_json::to_string(&msg).unwrap();
        let parsed: PluginToHost = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn shutdown_roundtrip() {
        let msg = HostToPlugin::Shutdown;
        let line = serde_json::to_string(&msg).unwrap();
        assert_eq!(line, r#"{"kind":"shutdown"}"#);
        let parsed: HostToPlugin = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn unknown_kind_fails_deserialization() {
        let line = r#"{"kind":"bogus","id":1}"#;
        let result: Result<HostToPlugin, _> = serde_json::from_str(line);
        assert!(result.is_err());
    }

    #[test]
    fn single_line_serialization() {
        // Newlines inside string values must not break line framing.
        let msg = PluginToHost::ActionResultOk {
            id: 1,
            output: json!({"text": "line1\nline2"}),
        };
        let line = serde_json::to_string(&msg).unwrap();
        assert!(!line.contains('\n'), "serialized form must be single-line");
        let parsed: PluginToHost = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, msg);
    }
}
