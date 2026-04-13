//! Process-based sandbox for community plugins using the duplex v2 protocol.
//!
//! Slice 1c (2026-04-13): plugin processes are **long-lived**. On the first
//! call, `ProcessSandbox` spawns the plugin binary, reads the handshake line
//! from its stdout, dials the announced UDS or Named Pipe, and stores the
//! resulting [`PluginHandle`] on the sandbox. Subsequent calls reuse that
//! handle, sending envelopes over the socket without respawning. A broken
//! connection (plugin crashed or exited) clears the handle and the next
//! request triggers a fresh spawn.
//!
//! The plugin-side event loop in `nebula-plugin-sdk::run_duplex` is still
//! sequential — one action at a time per plugin process. Slice 1d adds
//! concurrent multiplexed dispatch.
//!
//! Security enforcement (unchanged since slice 1b):
//! - `env_clear()` + explicit env allowlist
//! - `pre_exec` landlock + rlimits (Linux)
//! - stderr size limit for log capture
//! - `kill_on_drop` on the spawned child → plugin process dies with the sandbox

use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use nebula_action::{ActionError, ActionMetadata, result::ActionResult};
use nebula_plugin_sdk::{
    protocol::{HostToPlugin, PluginToHost},
    transport::{self, PluginStream},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};

use crate::{
    SandboxRunner,
    capabilities::{Capability, PluginCapabilities},
    runner::SandboxedContext,
};

/// Maximum stderr bytes captured for logging (64 KB). stdout is parsed
/// line-by-line for the handshake so has no fixed cap.
const MAX_STDERR_BYTES: usize = 64 * 1024;

/// Timeout for reading the plugin's handshake line from stdout.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

/// Correlation id used for the single envelope sent per invocation.
///
/// Slice 1c still does one envelope exchange at a time per call. Slice 1d's
/// concurrent dispatch assigns unique ids across multiple in-flight calls.
const ONE_SHOT_ID: u64 = 1;

/// Process sandbox: spawns the plugin binary once and keeps the connection
/// alive for the lifetime of this sandbox instance.
///
/// Each `ProcessSandbox` owns a long-lived [`PluginHandle`] behind a
/// `Mutex`. The first invocation spawns the child and dials the socket;
/// subsequent invocations reuse the same handle. A connection error on
/// write or read invalidates the handle and the next call respawns.
pub struct ProcessSandbox {
    /// Path to the plugin binary.
    binary: PathBuf,
    /// Per-call timeout (envelope round-trip wall clock).
    timeout: Duration,
    /// Capabilities granted to this plugin.
    capabilities: PluginCapabilities,
    /// Long-lived handle to the spawned plugin process. Serialized via the
    /// mutex — slice 1c is sequential per sandbox instance. Slice 1d can
    /// replace this with a lock-free handle once concurrent dispatch lands.
    handle: Mutex<Option<PluginHandle>>,
}

/// Live connection to a running plugin process.
///
/// Owns the spawned [`Child`] and the two halves of the accepted stream.
/// When dropped, `kill_on_drop(true)` on the child ensures the OS process
/// is terminated; the socket/pipe is released by `PluginStream`'s cleanup
/// guard on the plugin side.
struct PluginHandle {
    /// Kept alive for `kill_on_drop` — dropping this struct SIGKILLs the
    /// child. Read nowhere; the underscore prefix silences dead-code
    /// warnings.
    _child: Child,
    stream: PluginStream,
    line_buf: String,
}

impl PluginHandle {
    async fn send_envelope(&mut self, envelope: &HostToPlugin) -> Result<(), ActionError> {
        let encoded = serde_json::to_string(envelope)
            .map_err(|e| ActionError::fatal(format!("envelope serialization: {e}")))?;
        self.stream
            .write_all(encoded.as_bytes())
            .await
            .map_err(|e| ActionError::fatal(format!("plugin write error: {e}")))?;
        self.stream
            .write_all(b"\n")
            .await
            .map_err(|e| ActionError::fatal(format!("plugin write newline: {e}")))?;
        self.stream
            .flush()
            .await
            .map_err(|e| ActionError::fatal(format!("plugin flush: {e}")))?;
        Ok(())
    }

    async fn recv_envelope(&mut self) -> Result<PluginToHost, ActionError> {
        // Byte-at-a-time to avoid maintaining a separate BufReader that
        // would own the stream. Line volume per invocation is low.
        self.line_buf.clear();
        let mut byte = [0u8; 1];
        loop {
            let n = self
                .stream
                .read(&mut byte)
                .await
                .map_err(|e| ActionError::fatal(format!("plugin read error: {e}")))?;
            if n == 0 {
                return Err(ActionError::fatal(
                    "plugin closed transport without sending a response envelope",
                ));
            }
            if byte[0] == b'\n' {
                break;
            }
            self.line_buf.push(byte[0] as char);
        }
        let trimmed = self.line_buf.trim();
        serde_json::from_str::<PluginToHost>(trimmed)
            .map_err(|e| ActionError::fatal(format!("plugin sent malformed envelope: {e}")))
    }
}

impl ProcessSandbox {
    /// Create a new process sandbox for a plugin binary.
    #[must_use]
    pub fn new(binary: PathBuf, timeout: Duration, capabilities: PluginCapabilities) -> Self {
        Self {
            binary,
            timeout,
            capabilities,
            handle: Mutex::new(None),
        }
    }

    /// Invoke an action and return the plugin's response envelope.
    pub(crate) async fn call_action(
        &self,
        action_key: &str,
        input: serde_json::Value,
    ) -> Result<PluginToHost, ActionError> {
        let request = HostToPlugin::ActionInvoke {
            id: ONE_SHOT_ID,
            action_key: action_key.to_owned(),
            input,
        };
        self.dispatch_envelope(request).await
    }

    /// Query plugin metadata via a `MetadataRequest` envelope.
    pub async fn get_metadata(&self) -> Result<PluginToHost, ActionError> {
        let request = HostToPlugin::MetadataRequest { id: ONE_SHOT_ID };
        self.dispatch_envelope(request).await
    }

    /// Core long-lived dispatch. Reuses the cached [`PluginHandle`] if any,
    /// spawns fresh otherwise. On transport error, clears the handle and
    /// retries once.
    async fn dispatch_envelope(&self, envelope: HostToPlugin) -> Result<PluginToHost, ActionError> {
        let first_attempt = self.try_dispatch(envelope.clone()).await;
        if first_attempt.is_ok() {
            return first_attempt;
        }
        // Clear the stale handle and retry once with a fresh spawn.
        *self.handle.lock().await = None;
        self.try_dispatch(envelope).await
    }

    async fn try_dispatch(&self, envelope: HostToPlugin) -> Result<PluginToHost, ActionError> {
        let mut guard = self.handle.lock().await;
        if guard.is_none() {
            *guard = Some(self.spawn_and_dial().await?);
        }
        let handle = guard.as_mut().expect("handle set above");

        // Round-trip the envelope with a per-call timeout.
        let envelope_tag = match &envelope {
            HostToPlugin::ActionInvoke { .. } => "action_invoke",
            HostToPlugin::MetadataRequest { .. } => "metadata_request",
            _ => "other",
        };

        let result = tokio::time::timeout(self.timeout, async {
            handle.send_envelope(&envelope).await?;
            handle.recv_envelope().await
        })
        .await;

        match result {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => {
                // Transport/protocol error — invalidate the handle so the
                // next call respawns.
                *guard = None;
                Err(e)
            }
            Err(_) => {
                // Timeout — also invalidate; we don't know if the plugin is
                // still processing and we can't safely reuse the connection.
                *guard = None;
                Err(ActionError::retryable(format!(
                    "plugin {} timed out on {envelope_tag} after {:?}",
                    self.binary.display(),
                    self.timeout
                )))
            }
        }
    }

    /// Spawn the plugin binary, read and parse its handshake line, dial the
    /// announced transport, and return a fresh [`PluginHandle`].
    async fn spawn_and_dial(&self) -> Result<PluginHandle, ActionError> {
        // Build allowed env vars from capabilities.
        let allowed_env: Vec<(String, String)> = self
            .capabilities
            .list()
            .iter()
            .filter_map(|cap| match cap {
                Capability::Env { keys } => Some(keys.clone()),
                _ => None,
            })
            .flatten()
            .filter_map(|key| std::env::var(&key).ok().map(|val| (key, val)))
            .collect();

        let mut cmd = Command::new(&self.binary);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .env_clear()
            .envs(allowed_env);

        // Apply OS-level sandbox in child process before exec (Linux only).
        #[cfg(target_os = "linux")]
        {
            let caps_json = serde_json::to_string(&self.capabilities)
                .map_err(|e| ActionError::fatal(format!("capabilities serialization: {e}")))?;

            // SAFETY: pre_exec runs between fork() and exec() in the child.
            // We only call async-signal-safe operations (landlock, setrlimit).
            #[allow(unsafe_code)]
            unsafe {
                cmd.pre_exec(move || {
                    let caps: PluginCapabilities = serde_json::from_str(&caps_json)
                        .map_err(|e| std::io::Error::other(format!("capability parse: {e}")))?;
                    crate::os_sandbox::apply_sandbox(&caps)
                        .map_err(|e| std::io::Error::other(format!("sandbox setup: {e}")))
                });
            }
        }

        let mut child = cmd.spawn().map_err(|e| {
            ActionError::fatal(format!(
                "failed to spawn plugin {}: {e}",
                self.binary.display()
            ))
        })?;

        // Read handshake line from child stdout with timeout.
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ActionError::fatal("failed to open plugin stdout"))?;
        let mut stdout_reader = BufReader::new(stdout);
        let mut handshake_line = String::new();

        let read_result = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            stdout_reader.read_line(&mut handshake_line).await
        })
        .await;

        // Drain stderr into a log line (best effort).
        let stderr_buf = if let Some(mut stderr) = child.stderr.take() {
            let mut buf = vec![0u8; MAX_STDERR_BYTES];
            let n = stderr.read(&mut buf).await.unwrap_or(0);
            buf.truncate(n);
            buf
        } else {
            Vec::new()
        };
        if !stderr_buf.is_empty() {
            let stderr = sanitize_plugin_string(&String::from_utf8_lossy(&stderr_buf));
            tracing::debug!(
                plugin = %self.binary.display(),
                stderr = %stderr,
                "plugin stderr captured during spawn"
            );
        }

        let n = read_result
            .map_err(|_| {
                ActionError::fatal(format!(
                    "plugin {} handshake timeout after {HANDSHAKE_TIMEOUT:?}",
                    self.binary.display()
                ))
            })?
            .map_err(|e| {
                ActionError::fatal(format!(
                    "plugin {} handshake read error: {e}",
                    self.binary.display()
                ))
            })?;
        if n == 0 {
            return Err(ActionError::fatal(format!(
                "plugin {} exited before printing handshake line",
                self.binary.display()
            )));
        }

        tracing::debug!(
            plugin = %self.binary.display(),
            handshake = %handshake_line.trim(),
            "plugin handshake received"
        );

        // Dial the announced transport.
        let stream = transport::dial(handshake_line.trim())
            .await
            .map_err(|e| ActionError::fatal(format!("plugin transport dial failed: {e}")))?;

        Ok(PluginHandle {
            _child: child,
            stream,
            line_buf: String::with_capacity(512),
        })
    }
}

#[async_trait]
impl SandboxRunner for ProcessSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        context.check_cancelled()?;

        let action_key = metadata.key.as_str();

        tracing::debug!(
            action_key = %action_key,
            plugin = %self.binary.display(),
            "executing action in process sandbox"
        );

        let envelope = self.call_action(action_key, input).await?;
        match envelope {
            PluginToHost::ActionResultOk { output, .. } => Ok(ActionResult::success(output)),
            PluginToHost::ActionResultError {
                code,
                message,
                retryable,
                ..
            } => {
                let msg = sanitize_plugin_string(&format!("{code}: {message}"));
                if retryable {
                    Err(ActionError::retryable(msg))
                } else {
                    Err(ActionError::fatal(msg))
                }
            }
            other => Err(ActionError::fatal(format!(
                "plugin returned unexpected envelope (expected ActionResult*, got {})",
                envelope_kind(&other)
            ))),
        }
    }
}

/// Drop the cached handle on sandbox drop so the child is killed promptly.
///
/// `kill_on_drop(true)` on the spawned `Command` handles this at the OS
/// level — the destructor of `PluginHandle.child` sends SIGKILL. We add no
/// extra cleanup here; the `Arc<ProcessSandbox>` in the engine's handler
/// table owns the lifetime.
impl Drop for ProcessSandbox {
    fn drop(&mut self) {
        tracing::debug!(
            plugin = %self.binary.display(),
            "ProcessSandbox dropped; plugin child will be killed by kill_on_drop"
        );
    }
}

fn envelope_kind(env: &PluginToHost) -> &'static str {
    match env {
        PluginToHost::ActionResultOk { .. } => "action_result_ok",
        PluginToHost::ActionResultError { .. } => "action_result_error",
        PluginToHost::RpcCall { .. } => "rpc_call",
        PluginToHost::Log { .. } => "log",
        PluginToHost::MetadataResponse { .. } => "metadata_response",
    }
}

fn sanitize_plugin_string(s: &str) -> String {
    s.chars()
        .take(1024)
        .map(|c| if c.is_control() && c != '\n' { '?' } else { c })
        .collect()
}
