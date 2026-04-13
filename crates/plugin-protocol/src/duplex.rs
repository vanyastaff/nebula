//! Duplex protocol (v2): bidirectional envelope-based message stream between
//! host and plugin over stdio.
//!
//! This module defines the line-delimited JSON envelope format used by the
//! Phase 1 plugin broker. Each line of stdin (host→plugin) and stdout
//! (plugin→host) is one JSON object tagged by `kind`. See
//! `docs/plans/2026-04-13-sandbox-phase1-broker.md` for the full architecture.
//!
//! The one-shot v1 protocol ([`crate::PluginRequest`] / [`crate::PluginResponse`])
//! is kept for backward compatibility with the current `ProcessSandbox` and
//! will be removed once `ProcessSandbox` is rewritten to use this duplex
//! protocol (slice 1b of Phase 1).
//!
//! ## Framing
//!
//! - One message per line. Newlines inside strings must be escaped (standard JSON serialization
//!   with `serde_json::to_string` never produces a raw newline inside a string literal, so this is
//!   automatic).
//! - Reader must check `\n` terminators and parse the line as JSON.
//! - Writer must append `\n` after the JSON and flush.

use serde::{Deserialize, Serialize};

/// Duplex protocol version. Bumped when wire format changes incompatibly.
pub const DUPLEX_PROTOCOL_VERSION: u32 = 2;

/// Message from host to plugin.
///
/// Tagged by `kind`. Correlation IDs (`id`) are host-assigned `u64` values that
/// the plugin echoes back in the corresponding response message. IDs are
/// unique within a single plugin process lifetime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
        /// Unique plugin key (e.g., `"com.author.telegram"`).
        plugin_key: String,
        /// Semver plugin version string.
        plugin_version: String,
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

/// Describes one action offered by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionDescriptor {
    /// Full action key (e.g., `"telegram.send_message"`).
    pub key: String,
    /// Human-readable action name.
    pub name: String,
    /// Optional human-readable description.
    #[serde(default)]
    pub description: String,
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

        let resp = PluginToHost::MetadataResponse {
            id: 1,
            protocol_version: DUPLEX_PROTOCOL_VERSION,
            plugin_key: "com.author.echo".into(),
            plugin_version: "1.0.0".into(),
            actions: vec![ActionDescriptor {
                key: "echo".into(),
                name: "Echo".into(),
                description: "Echoes input".into(),
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
