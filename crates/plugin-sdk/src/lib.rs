#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Plugin SDK
//!
//! Ergonomic API for authoring community plugins against the Nebula duplex
//! broker protocol. Plugin authors implement [`PluginHandler`] and call
//! [`run_duplex`] from `main` — the SDK handles the wire protocol, line
//! framing, and dispatch.
//!
//! The wire envelope types live in the [`protocol`] submodule. Plugin authors
//! never touch them directly; the host (`nebula-sandbox`) imports them to
//! (de)serialize messages over the transport.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use nebula_plugin_sdk::{PluginCtx, PluginError, PluginHandler, PluginMeta, run_duplex};
//! use serde_json::Value;
//!
//! struct Echo;
//!
//! #[async_trait::async_trait]
//! impl PluginHandler for Echo {
//!     fn metadata(&self) -> PluginMeta {
//!         PluginMeta::new("com.example.echo", "0.1.0").with_action(
//!             "echo",
//!             "Echo",
//!             "Echoes the input",
//!         )
//!     }
//!
//!     async fn execute(
//!         &self,
//!         _ctx: &PluginCtx,
//!         _action_key: &str,
//!         input: Value,
//!     ) -> Result<Value, PluginError> {
//!         Ok(input)
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     run_duplex(Echo).await.unwrap();
//! }
//! ```
//!
//! ## Slice 1a scope
//!
//! Slice 1a of Phase 1 implements the **duplex JSON envelope protocol over
//! stdio** without gRPC / TLS / UDS. The SDK surface is deliberately
//! conservative: plugin authors see [`PluginHandler`] + [`PluginCtx`] only.
//! Future slices extend [`PluginCtx`] with `.network().http(...)`,
//! `.credentials().get(...)`, etc. via broker RPCs.
//!
//! Actions are dispatched **sequentially** within a single plugin process —
//! concurrent invocation support lands in slice 1b when message IDs enable
//! multiplexing.

use std::sync::Arc;

use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::protocol::{ActionDescriptor, DUPLEX_PROTOCOL_VERSION, HostToPlugin, PluginToHost};

pub mod protocol;
pub mod transport;

/// Error returned from a [`PluginHandler::execute`] call.
#[derive(Debug, Clone, Error)]
#[error("{code}: {message}")]
pub struct PluginError {
    /// Machine-readable error code (e.g., `"VALIDATION"`, `"TIMEOUT"`).
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Whether the error is transient and the host may retry.
    pub retryable: bool,
}

impl PluginError {
    /// Create a fatal (non-retryable) error.
    #[must_use]
    pub fn fatal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }

    /// Create a retryable error.
    #[must_use]
    pub fn retryable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: true,
        }
    }
}

/// Plugin context passed to [`PluginHandler::execute`].
///
/// Slice 1a: placeholder with no methods. Slice 1d+ adds broker RPC accessors
/// (`ctx.network()`, `ctx.credentials()`, `ctx.log()`, etc.) that issue
/// [`PluginToHost::RpcCall`] envelopes and await responses.
#[derive(Debug, Clone)]
pub struct PluginCtx {
    _priv: (),
}

impl PluginCtx {
    fn new() -> Self {
        Self { _priv: () }
    }
}

/// Plugin metadata builder. Shape is deliberately small in slice 1a; more
/// fields (credential slots, resource slots, parameter schemas) land in
/// slice 1e when derive-macros generate them from action types.
#[derive(Debug, Clone)]
pub struct PluginMeta {
    key: String,
    version: String,
    actions: Vec<ActionDescriptor>,
}

impl PluginMeta {
    /// Create a new metadata builder with required fields.
    #[must_use]
    pub fn new(key: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            version: version.into(),
            actions: Vec::new(),
        }
    }

    /// Add an action descriptor.
    #[must_use]
    pub fn with_action(
        mut self,
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        self.actions.push(ActionDescriptor {
            key: key.into(),
            name: name.into(),
            description: description.into(),
        });
        self
    }
}

/// Trait for plugin implementations.
///
/// Plugin authors implement this trait on a struct representing their plugin
/// and pass it to [`run_duplex`] from `main`.
#[async_trait::async_trait]
pub trait PluginHandler: Send + Sync + 'static {
    /// Return plugin metadata (key, version, actions).
    fn metadata(&self) -> PluginMeta;

    /// Execute an action.
    ///
    /// Called by the SDK whenever the host sends [`HostToPlugin::ActionInvoke`].
    /// The returned `Value` is serialized into the matching
    /// [`PluginToHost::ActionResultOk`]; a [`PluginError`] becomes
    /// [`PluginToHost::ActionResultError`].
    async fn execute(
        &self,
        ctx: &PluginCtx,
        action_key: &str,
        input: Value,
    ) -> Result<Value, PluginError>;
}

/// Run the plugin's duplex event loop.
///
/// # Lifecycle
///
/// 1. Binds a transport listener (Unix domain socket on Linux/macOS, named pipe on Windows) via
///    [`transport::bind_listener`].
/// 2. Prints the handshake line to **stdout** and flushes.
/// 3. Waits for exactly one incoming connection from the host.
/// 4. Runs the event loop over the accepted stream until the host closes the connection or sends
///    [`HostToPlugin::Shutdown`].
///
/// stdout is used only for the handshake line. After the connection is
/// accepted, all protocol traffic flows over the socket/pipe; the plugin
/// may still write diagnostic lines to stderr, which the host scrapes
/// into its logger.
///
/// # Behaviour inside the event loop
///
/// - Line-delimited JSON envelopes, one [`HostToPlugin`] per `\n`.
/// - Dispatches [`HostToPlugin::ActionInvoke`] to [`PluginHandler::execute`].
/// - Dispatches [`HostToPlugin::MetadataRequest`] to [`PluginHandler::metadata`].
/// - Ignores [`HostToPlugin::Cancel`] / [`HostToPlugin::RpcResponseOk`] /
///   [`HostToPlugin::RpcResponseError`] in slice 1c (concurrent dispatch and broker RPC flow land
///   in slice 1d).
/// - Exits cleanly on stream EOF or [`HostToPlugin::Shutdown`].
/// - Malformed JSON lines are logged via `tracing::warn!` and skipped; the loop continues. The host
///   and plugin stay in sync because every legit envelope is self-contained.
///
/// Slice 1c keeps plugin-side dispatch **sequential** — one action at a
/// time, head-of-line blocking. Slice 1d adds `tokio::spawn` per invocation
/// with a writer channel, so RPC calls can interleave with action execution.
///
/// # Errors
///
/// Returns [`std::io::Error`] on transport bind / accept failures or on
/// stdout handshake write failures. Read/write errors on the accepted
/// stream cause a clean exit (treated as "host dropped the connection").
pub async fn run_duplex<H: PluginHandler>(handler: H) -> std::io::Result<()> {
    let handler = Arc::new(handler);

    // Bind listener and emit handshake line on stdout.
    let (listener, handshake) = transport::bind_listener()?;
    {
        let mut stdout = tokio::io::stdout();
        stdout.write_all(handshake.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    // Wait for the host to dial.
    let stream = listener.accept().await?;
    run_event_loop(stream, handler).await
}

async fn run_event_loop<H: PluginHandler>(
    stream: transport::PluginStream,
    handler: Arc<H>,
) -> std::io::Result<()> {
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut writer = write_half;
    let ctx = PluginCtx::new();
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = match reader.read_line(&mut line).await {
            Ok(0) => {
                tracing::debug!("plugin: transport EOF, exiting");
                return Ok(());
            },
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %e, "plugin: transport read error, exiting");
                return Ok(());
            },
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let msg: HostToPlugin = match serde_json::from_str(trimmed) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    line_len = bytes,
                    "plugin: failed to parse HostToPlugin envelope, skipping line",
                );
                continue;
            },
        };

        match msg {
            HostToPlugin::ActionInvoke {
                id,
                action_key,
                input,
            } => {
                let response = match handler.execute(&ctx, &action_key, input).await {
                    Ok(output) => PluginToHost::ActionResultOk { id, output },
                    Err(err) => PluginToHost::ActionResultError {
                        id,
                        code: err.code,
                        message: err.message,
                        retryable: err.retryable,
                    },
                };
                write_line(&mut writer, &response).await?;
            },
            HostToPlugin::MetadataRequest { id } => {
                let meta = handler.metadata();
                let response = PluginToHost::MetadataResponse {
                    id,
                    protocol_version: DUPLEX_PROTOCOL_VERSION,
                    plugin_key: meta.key,
                    plugin_version: meta.version,
                    actions: meta.actions,
                };
                write_line(&mut writer, &response).await?;
            },
            HostToPlugin::Shutdown => {
                tracing::debug!("plugin: received Shutdown, exiting");
                return Ok(());
            },
            HostToPlugin::Cancel { .. }
            | HostToPlugin::RpcResponseOk { .. }
            | HostToPlugin::RpcResponseError { .. } => {
                // Slice 1c: sequential dispatch means Cancel is a no-op
                // (the in-flight invocation blocks the loop anyway). Slice
                // 1d adds concurrent dispatch and pending-call tables and
                // routes these correctly.
            },
        }
    }
}

async fn write_line<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    msg: &PluginToHost,
) -> std::io::Result<()> {
    let encoded = serde_json::to_string(msg).map_err(std::io::Error::other)?;
    writer.write_all(encoded.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHandler;

    #[async_trait::async_trait]
    impl PluginHandler for TestHandler {
        fn metadata(&self) -> PluginMeta {
            PluginMeta::new("com.test.echo", "0.1.0").with_action(
                "echo",
                "Echo",
                "Returns input unchanged",
            )
        }

        async fn execute(
            &self,
            _ctx: &PluginCtx,
            action_key: &str,
            input: Value,
        ) -> Result<Value, PluginError> {
            match action_key {
                "echo" => Ok(input),
                other => Err(PluginError::fatal(
                    "UNKNOWN_ACTION",
                    format!("unknown action: {other}"),
                )),
            }
        }
    }

    #[test]
    fn plugin_error_builders() {
        let f = PluginError::fatal("X", "oops");
        assert_eq!(f.code, "X");
        assert!(!f.retryable);

        let r = PluginError::retryable("Y", "try again");
        assert!(r.retryable);
    }

    #[test]
    fn plugin_meta_builder() {
        let m = PluginMeta::new("p", "1.0")
            .with_action("a1", "A1", "first")
            .with_action("a2", "A2", "second");
        assert_eq!(m.key, "p");
        assert_eq!(m.version, "1.0");
        assert_eq!(m.actions.len(), 2);
        assert_eq!(m.actions[0].key, "a1");
    }

    #[tokio::test]
    async fn test_handler_execute_ok() {
        let h = TestHandler;
        let ctx = PluginCtx::new();
        let result = h
            .execute(&ctx, "echo", serde_json::json!({"x": 1}))
            .await
            .unwrap();
        assert_eq!(result, serde_json::json!({"x": 1}));
    }

    #[tokio::test]
    async fn test_handler_execute_unknown_action() {
        let h = TestHandler;
        let ctx = PluginCtx::new();
        let err = h
            .execute(&ctx, "nope", serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!(err.code, "UNKNOWN_ACTION");
        assert!(!err.retryable);
    }
}
