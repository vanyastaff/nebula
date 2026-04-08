#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Plugin Protocol
//!
//! Typed protocol for Nebula community plugins (process-isolated binaries).
//!
//! Plugin authors depend on this crate to implement the stdin/stdout JSON protocol.
//! The host (nebula-sandbox) uses the same types for (de)serialization.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use nebula_plugin_protocol::*;
//!
//! struct MyPlugin;
//!
//! impl PluginHandler for MyPlugin {
//!     fn metadata(&self) -> PluginMetadata {
//!         PluginMetadata::new("my_plugin", "My Plugin")
//!             .action("my_plugin.greet", "Greet", "Says hello")
//!     }
//!
//!     fn execute(&self, action: &str, input: Value) -> PluginResult {
//!         match action {
//!             "my_plugin.greet" => PluginResult::success(json!({"hello": "world"})),
//!             _ => PluginResult::error("UNKNOWN_ACTION", format!("unknown: {action}")),
//!         }
//!     }
//! }
//!
//! fn main() {
//!     nebula_plugin_protocol::run(MyPlugin);
//! }
//! ```

use serde::{Deserialize, Serialize};

/// Current protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Reserved action key for metadata requests.
pub const METADATA_ACTION: &str = "__metadata__";

// ── Request ──────────────────────────────────────────────────────────────

/// Request sent from host to plugin via stdin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRequest {
    /// Action to execute, or `"__metadata__"` for plugin info.
    pub action_key: String,
    /// Input data for the action.
    #[serde(default)]
    pub input: serde_json::Value,
}

// ── Response ─────────────────────────────────────────────────────────────

/// Response sent from plugin to host via stdout.
///
/// Uses tagged enum with `"status"` discriminator — unambiguous deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PluginResponse {
    /// Successful execution.
    Ok {
        /// Output data.
        output: serde_json::Value,
    },
    /// Failed execution.
    Error {
        /// Error code (e.g., "VALIDATION", "TIMEOUT", "UNKNOWN_ACTION").
        code: String,
        /// Human-readable error message.
        message: String,
        /// Whether the error is retryable.
        #[serde(default)]
        retryable: bool,
    },
}

// ── Metadata ─────────────────────────────────────────────────────────────

/// Plugin metadata returned in response to `__metadata__` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    /// Protocol version (must match host).
    pub protocol_version: u32,
    /// Unique plugin key.
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
    pub actions: Vec<ActionMeta>,
}

/// Describes an action provided by the plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionMeta {
    /// Full action key (e.g., "telegram.send_message").
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

impl PluginMetadata {
    /// Create metadata with required fields.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            key: key.into(),
            name: name.into(),
            version: 1,
            description: String::new(),
            actions: Vec::new(),
        }
    }

    /// Set description.
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set version.
    #[must_use]
    pub fn version(mut self, v: u32) -> Self {
        self.version = v;
        self
    }

    /// Add an action.
    #[must_use]
    pub fn action(
        mut self,
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        self.actions.push(ActionMeta {
            key: key.into(),
            name: name.into(),
            description: description.into(),
        });
        self
    }
}

// ── PluginResult helper ──────────────────────────────────────────────────

/// Convenience type for building plugin responses.
pub type PluginResult = PluginResponse;

impl PluginResponse {
    /// Success with output data.
    #[must_use]
    pub fn success(output: serde_json::Value) -> Self {
        Self::Ok { output }
    }

    /// Fatal error.
    #[must_use]
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }

    /// Retryable error.
    #[must_use]
    pub fn retryable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
            retryable: true,
        }
    }

    /// Unknown action error.
    #[must_use]
    pub fn unknown_action(key: &str) -> Self {
        Self::error("UNKNOWN_ACTION", format!("unknown action: {key}"))
    }
}

// ── Handler trait ────────────────────────────────────────────────────────

/// Trait for plugin implementations.
///
/// Implement this and pass to [`run()`] to handle the protocol automatically.
pub trait PluginHandler {
    /// Return plugin metadata (name, version, actions).
    fn metadata(&self) -> PluginMetadata;

    /// Execute an action with the given input.
    fn execute(&self, action_key: &str, input: serde_json::Value) -> PluginResult;
}

// ── Entry point ──────────────────────────────────────────────────────────

/// Run the plugin protocol loop.
///
/// Reads a JSON request from stdin, dispatches to the handler, writes
/// a JSON response to stdout. Call this from `main()`.
///
/// # Panics
///
/// Panics if stdin cannot be read or stdout cannot be written. This is
/// intentional — a plugin with broken I/O cannot function.
pub fn run(handler: impl PluginHandler) {
    let request: PluginRequest = match serde_json::from_reader(std::io::stdin()) {
        Ok(req) => req,
        Err(e) => {
            let response = PluginResponse::error("PROTOCOL_ERROR", format!("invalid request: {e}"));
            let json = serde_json::to_string(&response).expect("response serialization");
            println!("{json}");
            std::process::exit(1);
        }
    };

    let response = if request.action_key == METADATA_ACTION {
        let meta = handler.metadata();
        PluginResponse::success(serde_json::to_value(meta).expect("metadata serialization"))
    } else {
        handler.execute(&request.action_key, request.input)
    };

    let json = serde_json::to_string(&response).expect("response serialization");
    println!("{json}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn response_ok_serialization() {
        let resp = PluginResponse::success(json!({"hello": "world"}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""status":"ok""#));
        assert!(json.contains(r#""hello":"world""#));
    }

    #[test]
    fn response_error_serialization() {
        let resp = PluginResponse::error("TIMEOUT", "took too long");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""status":"error""#));
        assert!(json.contains(r#""code":"TIMEOUT""#));
        assert!(json.contains(r#""retryable":false"#));
    }

    #[test]
    fn response_retryable_serialization() {
        let resp = PluginResponse::retryable("RATE_LIMIT", "429");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""retryable":true"#));
    }

    #[test]
    fn response_deserialization_tagged() {
        let ok: PluginResponse =
            serde_json::from_str(r#"{"status":"ok","output":{"x":1}}"#).unwrap();
        assert!(matches!(ok, PluginResponse::Ok { .. }));

        let err: PluginResponse = serde_json::from_str(
            r#"{"status":"error","code":"FAIL","message":"boom","retryable":false}"#,
        )
        .unwrap();
        assert!(matches!(err, PluginResponse::Error { .. }));
    }

    #[test]
    fn response_no_ambiguity() {
        // This was the bug with untagged: {"output": null, "error": "..."} matched Success.
        // With tagged, this is unambiguous.
        let resp: Result<PluginResponse, _> =
            serde_json::from_str(r#"{"output": null, "error": "something"}"#);
        // Without "status" field, deserialization fails — correct behavior.
        assert!(resp.is_err());
    }

    #[test]
    fn metadata_builder() {
        let meta = PluginMetadata::new("telegram", "Telegram Bot")
            .description("Send messages")
            .version(2)
            .action("telegram.send", "Send Message", "Sends a message");

        assert_eq!(meta.protocol_version, PROTOCOL_VERSION);
        assert_eq!(meta.key, "telegram");
        assert_eq!(meta.version, 2);
        assert_eq!(meta.actions.len(), 1);
    }

    #[test]
    fn unknown_action_helper() {
        let resp = PluginResponse::unknown_action("bad.action");
        match resp {
            PluginResponse::Error { code, message, .. } => {
                assert_eq!(code, "UNKNOWN_ACTION");
                assert!(message.contains("bad.action"));
            }
            _ => panic!("expected error"),
        }
    }
}
