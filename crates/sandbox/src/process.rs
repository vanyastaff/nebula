//! Process-based sandbox for community plugins.
//!
//! Each plugin is a separate binary using the `nebula-plugin-protocol` crate.
//! Host communicates via stdin/stdout JSON with tagged `PluginResponse`.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use nebula_action::result::ActionResult;
use nebula_action::{ActionError, ActionMetadata};
use nebula_plugin_protocol::{PluginRequest, PluginResponse};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::SandboxRunner;
use crate::runner::SandboxedContext;

/// Maximum stdout size from a plugin (10 MB). Prevents DoS.
const MAX_STDOUT_BYTES: usize = 10 * 1024 * 1024;

/// Process sandbox: runs plugin actions as isolated child processes.
///
/// Each action execution spawns the plugin binary, sends a [`PluginRequest`]
/// via stdin, reads a [`PluginResponse`] from stdout.
pub struct ProcessSandbox {
    /// Path to the plugin binary.
    binary: PathBuf,
    /// Timeout for each action execution.
    timeout: Duration,
}

impl ProcessSandbox {
    /// Create a new process sandbox for a plugin binary.
    #[must_use]
    pub fn new(binary: PathBuf, timeout: Duration) -> Self {
        Self { binary, timeout }
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

        let mut child = Command::new(&self.binary)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
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

        // Wait for output with timeout.
        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                ActionError::retryable(format!("plugin timed out after {:?}", self.timeout))
            })?
            .map_err(|e| ActionError::fatal(format!("plugin process error: {e}")))?;

        // Log stderr if any.
        if !output.stderr.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!(
                plugin = %self.binary.display(),
                stderr = %stderr,
                "plugin stderr"
            );
        }

        // Check exit code.
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ActionError::fatal(format!(
                "plugin exited with {}: {}",
                output.status,
                stderr.trim()
            )));
        }

        // Enforce stdout size limit (prevents DoS from malicious plugins).
        if output.stdout.len() > MAX_STDOUT_BYTES {
            return Err(ActionError::fatal(format!(
                "plugin stdout exceeds {} bytes limit",
                MAX_STDOUT_BYTES
            )));
        }

        // Parse stdout.
        String::from_utf8(output.stdout)
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
                if retryable {
                    Err(ActionError::retryable(format!("{code}: {message}")))
                } else {
                    Err(ActionError::fatal(format!("{code}: {message}")))
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
