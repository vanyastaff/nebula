//! WASM sandbox implementation using extism.

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use extism::{Manifest, Plugin, Wasm};
use nebula_action::result::ActionResult;
use nebula_action::{ActionError, ActionMetadata};

use super::loader::WasmPluginMetadata;
use crate::SandboxRunner;
use crate::runner::SandboxedContext;

/// WASM sandbox: runs plugin actions in an extism/wasmtime sandbox.
///
/// Each `WasmSandbox` instance wraps one `.wasm` plugin file.
/// Actions are executed by calling the `execute` export with JSON.
pub struct WasmSandbox {
    /// The extism plugin instance.
    /// Mutex because extism::Plugin is not Sync.
    plugin: Mutex<Plugin>,
    /// Cached metadata from the plugin.
    metadata: WasmPluginMetadata,
}

impl WasmSandbox {
    /// Load a WASM plugin from a file path.
    pub fn from_file(path: &Path) -> Result<Self, ActionError> {
        let wasm = Wasm::file(path);
        let manifest = Manifest::new([wasm]);

        let mut plugin = Plugin::new(&manifest, [], true).map_err(|e| {
            ActionError::fatal(format!(
                "failed to load WASM plugin {}: {e}",
                path.display()
            ))
        })?;

        // Call metadata export to get plugin info.
        let metadata_json = plugin
            .call::<&str, &str>("metadata", "")
            .map_err(|e| ActionError::fatal(format!("metadata() call failed: {e}")))?;

        let metadata: WasmPluginMetadata = serde_json::from_str(metadata_json)
            .map_err(|e| ActionError::fatal(format!("invalid metadata JSON: {e}")))?;

        tracing::info!(
            plugin_key = %metadata.key,
            plugin_name = %metadata.name,
            actions = metadata.actions.len(),
            "loaded WASM plugin"
        );

        Ok(Self {
            plugin: Mutex::new(plugin),
            metadata,
        })
    }

    /// Get the cached plugin metadata.
    pub fn metadata(&self) -> &WasmPluginMetadata {
        &self.metadata
    }

    /// Execute an action by key with JSON input.
    pub fn execute_action(
        &self,
        action_key: &str,
        input_json: &str,
    ) -> Result<String, ActionError> {
        let request = serde_json::json!({
            "action_key": action_key,
            "input": serde_json::from_str::<serde_json::Value>(input_json)
                .unwrap_or(serde_json::Value::Null),
        });
        let request_str = request.to_string();

        let mut plugin = self.plugin.lock().unwrap();
        let output = plugin
            .call::<&str, &str>("execute", &request_str)
            .map_err(|e| ActionError::fatal(format!("execute() failed: {e}")))?;

        Ok(output.to_owned())
    }
}

#[async_trait]
impl SandboxRunner for WasmSandbox {
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
            plugin = %self.metadata.key,
            "executing action in WASM sandbox"
        );

        // extism::Plugin is sync — run on blocking thread pool.
        let action_key = action_key.to_owned();

        let output_json =
            tokio::task::block_in_place(|| self.execute_action(&action_key, &input_json))?;

        let output: serde_json::Value = serde_json::from_str(&output_json)
            .map_err(|e| ActionError::fatal(format!("output deserialization: {e}")))?;

        Ok(ActionResult::success(output))
    }
}

// SAFETY: WasmSandbox needs Send + Sync for SandboxRunner.
// Plugin is behind Mutex (provides Sync), WasmPluginMetadata is Send + Sync.
#[allow(unsafe_code)]
unsafe impl Send for WasmSandbox {}
#[allow(unsafe_code)]
unsafe impl Sync for WasmSandbox {}
