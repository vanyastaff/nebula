//! Process-based sandbox for community plugins.
//!
//! Each plugin is a separate binary. The host communicates via stdin/stdout JSON.
//!
//! Protocol:
//! - Host writes a JSON request to stdin: `{"action_key": "...", "input": {...}}`
//! - Plugin writes a JSON response to stdout: `{"output": {...}}` or `{"error": {...}}`
//! - Plugin stderr goes to host tracing (debug level)

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use nebula_action::result::ActionResult;
use nebula_action::{ActionError, ActionMetadata};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::SandboxRunner;
use crate::runner::SandboxedContext;

/// Process sandbox: runs plugin actions as isolated child processes.
///
/// Each action execution spawns the plugin binary, sends JSON over stdin,
/// reads JSON from stdout. The child process is killed on timeout.
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
        input_json: &str,
    ) -> Result<String, ActionError> {
        let request = serde_json::json!({
            "action_key": action_key,
            "input": serde_json::from_str::<serde_json::Value>(input_json)
                .unwrap_or(serde_json::Value::Null),
        });

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

        // Parse stdout as JSON.
        let stdout = String::from_utf8(output.stdout)
            .map_err(|e| ActionError::fatal(format!("plugin output is not UTF-8: {e}")))?;

        Ok(stdout)
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
        let input_json = serde_json::to_string(&input)
            .map_err(|e| ActionError::fatal(format!("input serialization: {e}")))?;

        tracing::debug!(
            action_key = %action_key,
            plugin = %self.binary.display(),
            "executing action in process sandbox"
        );

        let output_json = self.call(action_key, &input_json).await?;

        // Parse the plugin response.
        let response: PluginResponse = serde_json::from_str(&output_json)
            .map_err(|e| ActionError::fatal(format!("invalid plugin response: {e}")))?;

        match response {
            PluginResponse::Success { output } => Ok(ActionResult::success(output)),
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

/// Response format from a plugin process.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum PluginResponse {
    /// Successful execution.
    Success {
        /// The action output.
        output: serde_json::Value,
    },
    /// Failed execution.
    Error {
        /// Error code.
        #[serde(default = "default_error_code")]
        code: String,
        /// Error message.
        #[serde(alias = "error")]
        message: String,
        /// Whether the error is retryable.
        #[serde(default)]
        retryable: bool,
    },
}

fn default_error_code() -> String {
    "PLUGIN_ERROR".to_owned()
}
