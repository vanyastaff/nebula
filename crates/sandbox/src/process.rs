//! Process-based sandbox for community plugins using the duplex v2 protocol.
//!
//! Each action execution spawns the plugin binary, sends one envelope over
//! stdin (either `ActionInvoke` for action execution or `MetadataRequest` for
//! discovery), reads the first valid response envelope from stdout, and waits
//! for the plugin to exit. Slice 1b still spawns per-call; slice 1d replaces
//! this with a long-lived `PluginSupervisor` that keeps plugin processes
//! alive across invocations with Reattach.
//!
//! Security enforcement:
//! - `env_clear()` — plugin only sees explicitly allowed env vars
//! - `pre_exec` — applies landlock (filesystem) + rlimits before plugin starts (Linux only)
//! - Stdout size limit — prevents OOM from malicious plugins
//! - Timeout + `kill_on_drop` — prevents infinite hangs

use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use nebula_action::{ActionError, ActionMetadata, result::ActionResult};
use nebula_plugin_protocol::duplex::{HostToPlugin, PluginToHost};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

use crate::{
    SandboxRunner,
    capabilities::{Capability, PluginCapabilities},
    runner::SandboxedContext,
};

/// Maximum stdout size from a plugin (10 MB). Prevents DoS.
const MAX_STDOUT_BYTES: u64 = 10 * 1024 * 1024;

/// Maximum stderr size to capture for logging (64 KB).
const MAX_STDERR_BYTES: usize = 64 * 1024;

/// Correlation id used for the single envelope sent per invocation.
///
/// Slice 1b keeps the one-shot spawn-per-call shape from slice 1a; we only
/// ever issue one envelope per plugin process lifetime, so the id is fixed.
/// Slice 1d's `PluginSupervisor` assigns unique ids across concurrent calls
/// to a long-lived plugin process.
const ONE_SHOT_ID: u64 = 1;

/// Process sandbox: runs plugin actions as isolated child processes.
///
/// Spawns the plugin binary per call, sends one [`HostToPlugin`] envelope,
/// reads the first [`PluginToHost`] envelope from stdout, waits for exit.
pub struct ProcessSandbox {
    /// Path to the plugin binary.
    binary: PathBuf,
    /// Timeout for each action execution.
    timeout: Duration,
    /// Capabilities granted to this plugin.
    capabilities: PluginCapabilities,
}

impl ProcessSandbox {
    /// Create a new process sandbox for a plugin binary.
    #[must_use]
    pub fn new(binary: PathBuf, timeout: Duration, capabilities: PluginCapabilities) -> Self {
        Self {
            binary,
            timeout,
            capabilities,
        }
    }

    /// Invoke an action and return the plugin's response envelope.
    ///
    /// Callers are expected to match on [`PluginToHost::ActionResultOk`] /
    /// [`PluginToHost::ActionResultError`]; any other variant signals a
    /// protocol violation.
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

    /// Query plugin metadata via the duplex [`HostToPlugin::MetadataRequest`]
    /// envelope. Used by plugin discovery.
    pub async fn get_metadata(&self) -> Result<PluginToHost, ActionError> {
        let request = HostToPlugin::MetadataRequest { id: ONE_SHOT_ID };
        self.dispatch_envelope(request).await
    }

    /// Core one-shot dispatch: spawn child, send envelope, read envelope, wait.
    async fn dispatch_envelope(&self, envelope: HostToPlugin) -> Result<PluginToHost, ActionError> {
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
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .env_clear() // SECURITY: start with empty environment
            .envs(allowed_env); // only pass granted env vars

        // Apply OS-level sandbox in child process before exec (Linux only).
        #[cfg(target_os = "linux")]
        {
            let caps_json = serde_json::to_string(&self.capabilities)
                .map_err(|e| ActionError::fatal(format!("capabilities serialization: {e}")))?;

            // SAFETY: pre_exec runs between fork() and exec() in the child.
            // We only call async-signal-safe operations (landlock, setrlimit).
            // caps_json is a moved String — safe in single-threaded child context.
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

        // Write the envelope to stdin as one line.
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| ActionError::fatal("failed to open plugin stdin"))?;
        let encoded = serde_json::to_string(&envelope)
            .map_err(|e| ActionError::fatal(format!("envelope serialization: {e}")))?;
        stdin
            .write_all(encoded.as_bytes())
            .await
            .map_err(|e| ActionError::fatal(format!("failed to write plugin stdin: {e}")))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| ActionError::fatal(format!("failed to write newline: {e}")))?;
        // Closing stdin signals end-of-input → plugin's `run_duplex` loop sees
        // EOF and exits after responding. Slice 1d keeps stdin open.
        stdin
            .shutdown()
            .await
            .map_err(|e| ActionError::fatal(format!("failed to close plugin stdin: {e}")))?;

        // Read stdout with size limit (prevents OOM from malicious plugins).
        let stdout_handle = child
            .stdout
            .take()
            .ok_or_else(|| ActionError::fatal("failed to open plugin stdout"))?;
        let limited = stdout_handle.take(MAX_STDOUT_BYTES);
        let mut reader = BufReader::new(limited);

        let binary_name = self.binary.display().to_string();
        let read_result =
            tokio::time::timeout(self.timeout, read_envelope(&mut reader, &binary_name)).await;

        // Capture stderr (limited) for logging, regardless of outcome.
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
                "plugin stderr"
            );
        }

        let envelope = match read_result {
            Ok(Ok(env)) => env,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(ActionError::retryable(format!(
                    "plugin timed out after {:?}",
                    self.timeout
                )));
            }
        };

        // Wait for child to exit cleanly within a short grace window.
        // If the plugin is misbehaving and hasn't exited, `kill_on_drop` will
        // SIGKILL it when `child` drops.
        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;

        Ok(envelope)
    }
}

/// Read lines from the plugin's stdout until a parseable [`PluginToHost`]
/// envelope is found. Empty and malformed lines are skipped with a
/// `tracing::warn!`. Returns an error on EOF without a valid envelope.
async fn read_envelope<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
    binary_name: &str,
) -> Result<PluginToHost, ActionError> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| ActionError::fatal(format!("plugin stdout read error: {e}")))?;
        if n == 0 {
            return Err(ActionError::fatal(format!(
                "plugin {binary_name} closed stdout without sending a response envelope"
            )));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<PluginToHost>(trimmed) {
            Ok(env) => return Ok(env),
            Err(e) => {
                tracing::warn!(
                    plugin = %binary_name,
                    error = %e,
                    "malformed envelope from plugin, skipping line"
                );
                continue;
            }
        }
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

/// Returns the discriminant name of a `PluginToHost` envelope for error
/// messages. Kept in sync with the enum variants.
fn envelope_kind(env: &PluginToHost) -> &'static str {
    match env {
        PluginToHost::ActionResultOk { .. } => "action_result_ok",
        PluginToHost::ActionResultError { .. } => "action_result_error",
        PluginToHost::RpcCall { .. } => "rpc_call",
        PluginToHost::Log { .. } => "log",
        PluginToHost::MetadataResponse { .. } => "metadata_response",
    }
}

/// Sanitize a plugin-sourced string for safe logging/display.
/// Removes control characters (except newline) and truncates.
fn sanitize_plugin_string(s: &str) -> String {
    s.chars()
        .take(1024)
        .map(|c| if c.is_control() && c != '\n' { '?' } else { c })
        .collect()
}
