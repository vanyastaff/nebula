//! Process-based sandbox for community plugins.
//!
//! Each plugin is a separate binary using the `nebula-plugin-protocol` crate.
//! Host communicates via stdin/stdout JSON with tagged `PluginResponse`.
//!
//! Security enforcement:
//! - `env_clear()` — plugin only sees explicitly allowed env vars
//! - `pre_exec` — applies landlock (filesystem) + rlimits before plugin starts
//! - Stdout size limit — prevents OOM from malicious plugins
//! - Timeout + kill_on_drop — prevents infinite hangs

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use nebula_action::result::ActionResult;
use nebula_action::{ActionError, ActionMetadata};
use nebula_plugin_protocol::{PluginRequest, PluginResponse};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::SandboxRunner;
use crate::capabilities::{Capability, PluginCapabilities};
use crate::runner::SandboxedContext;

/// Maximum stdout size from a plugin (10 MB). Prevents DoS.
const MAX_STDOUT_BYTES: u64 = 10 * 1024 * 1024;

/// Maximum stderr size to capture for logging (64 KB).
const MAX_STDERR_BYTES: usize = 64 * 1024;

/// Process sandbox: runs plugin actions as isolated child processes.
///
/// Each action execution spawns the plugin binary, sends a [`PluginRequest`]
/// via stdin, reads a [`PluginResponse`] from stdout.
///
/// Capabilities are enforced via:
/// - `env_clear()` + env allowlist
/// - `pre_exec` landlock + rlimits (Linux)
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

    /// Execute an action via the child process.
    pub(crate) async fn call(
        &self,
        action_key: &str,
        input: serde_json::Value,
    ) -> Result<String, ActionError> {
        let request = PluginRequest {
            action_key: action_key.to_owned(),
            input,
        };

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
                    let caps: PluginCapabilities =
                        serde_json::from_str(&caps_json).map_err(|e| {
                            std::io::Error::other(format!("capability parse: {e}"))
                        })?;
                    crate::os_sandbox::apply_sandbox(&caps).map_err(|e| {
                        std::io::Error::other(format!("sandbox setup: {e}"))
                    })
                });
            }
        }

        let mut child = cmd.spawn().map_err(|e| {
            ActionError::fatal(format!(
                "failed to spawn plugin {}: {e}",
                self.binary.display()
            ))
        })?;

        // Write request to stdin.
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| ActionError::fatal("failed to open plugin stdin"))?;
        let request_bytes = serde_json::to_vec(&request)
            .map_err(|e| ActionError::fatal(format!("request serialization: {e}")))?;
        stdin
            .write_all(&request_bytes)
            .await
            .map_err(|e| ActionError::fatal(format!("failed to write to plugin stdin: {e}")))?;
        stdin
            .shutdown()
            .await
            .map_err(|e| ActionError::fatal(format!("failed to close plugin stdin: {e}")))?;

        // Read stdout with size limit (prevents OOM from malicious plugins).
        let stdout_handle = child
            .stdout
            .take()
            .ok_or_else(|| ActionError::fatal("failed to open plugin stdout"))?;
        let mut limited_reader = stdout_handle.take(MAX_STDOUT_BYTES);
        let mut stdout_buf = Vec::new();

        let read_result =
            tokio::time::timeout(self.timeout, limited_reader.read_to_end(&mut stdout_buf)).await;

        // Also capture stderr (limited, for logging).
        let stderr_buf = if let Some(mut stderr) = child.stderr.take() {
            let mut buf = vec![0u8; MAX_STDERR_BYTES];
            let n = stderr.read(&mut buf).await.unwrap_or(0);
            buf.truncate(n);
            buf
        } else {
            Vec::new()
        };

        // Log stderr (sanitized).
        if !stderr_buf.is_empty() {
            let stderr = sanitize_plugin_string(&String::from_utf8_lossy(&stderr_buf));
            tracing::debug!(
                plugin = %self.binary.display(),
                stderr = %stderr,
                "plugin stderr"
            );
        }

        // Handle timeout.
        let bytes_read = read_result
            .map_err(|_| {
                ActionError::retryable(format!("plugin timed out after {:?}", self.timeout))
            })?
            .map_err(|e| ActionError::fatal(format!("plugin stdout read error: {e}")))?;

        // Wait for child to exit.
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .map_err(|_| ActionError::fatal("plugin did not exit after stdout closed"))?
            .map_err(|e| ActionError::fatal(format!("plugin wait error: {e}")))?;

        if !status.success() {
            let stderr = sanitize_plugin_string(&String::from_utf8_lossy(&stderr_buf));
            return Err(ActionError::fatal(format!(
                "plugin exited with {status}: {stderr}"
            )));
        }

        // Check if we hit the size limit.
        if bytes_read as u64 >= MAX_STDOUT_BYTES {
            return Err(ActionError::fatal(format!(
                "plugin stdout exceeds {MAX_STDOUT_BYTES} byte limit"
            )));
        }

        String::from_utf8(stdout_buf)
            .map_err(|e| ActionError::fatal(format!("plugin output is not UTF-8: {e}")))
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

        let output_json = self.call(action_key, input).await?;

        // Parse the tagged plugin response.
        let response: PluginResponse = serde_json::from_str(&output_json).map_err(|e| {
            ActionError::fatal(format!(
                "invalid plugin response: {e}\nraw output: {}",
                truncate_str(&output_json, 500)
            ))
        })?;

        match response {
            PluginResponse::Ok { output } => Ok(ActionResult::success(output)),
            PluginResponse::Error {
                code,
                message,
                retryable,
            } => {
                let msg = sanitize_plugin_string(&format!("{code}: {message}"));
                if retryable {
                    Err(ActionError::retryable(msg))
                } else {
                    Err(ActionError::fatal(msg))
                }
            }
        }
    }
}

/// Truncate a string safely at char boundary.
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
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
