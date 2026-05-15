//! ProcessSandboxHandler — bridges ProcessSandbox into ActionRegistry.
//!
//! Implements `StatelessHandler` so the engine can call community plugin
//! actions through the process sandbox transparently.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessHandler};
use nebula_sandbox::ProcessSandbox;

use crate::sandbox_bridge::sandbox_error_to_action_error;

/// Wraps a [`ProcessSandbox`] as a [`StatelessHandler`].
///
/// Each `ProcessSandboxHandler` represents one action from a community plugin.
/// When the engine calls `execute()`, the request is routed through the
/// sandbox's long-lived plugin process using the duplex envelope transport
/// (ADR 0006; handshake + dialed socket), not direct stdin/stdout action
/// invocation. The transport returns a `SandboxError`; this handler maps it
/// to the `ActionError` taxonomy at the `StatelessHandler` boundary.
///
/// `metadata.base.key` is the **namespaced** host-side key
/// (`<plugin>.<local>`); the plugin's own `PluginHandler::execute` matches
/// on the **un-namespaced local key**. The local key is therefore stored
/// separately and is what gets sent over the transport — sending the
/// namespaced key would make the plugin reject the call with
/// `UNKNOWN_ACTION`.
pub struct ProcessSandboxHandler {
    sandbox: Arc<ProcessSandbox>,
    metadata: ActionMetadata,
    /// The un-namespaced action key the plugin matches on in its own
    /// `PluginHandler::execute` (the raw wire `ActionDescriptor.key`). Sent
    /// over the transport instead of the namespaced `metadata.base.key`.
    local_key: String,
}

impl ProcessSandboxHandler {
    /// Create a new handler for an action backed by a process sandbox.
    ///
    /// `local_key` is the un-namespaced key the plugin matches on (the raw
    /// wire `ActionDescriptor.key`), distinct from the namespaced
    /// `metadata.base.key`.
    pub fn new(sandbox: Arc<ProcessSandbox>, metadata: ActionMetadata, local_key: String) -> Self {
        Self {
            sandbox,
            metadata,
            local_key,
        }
    }
}

#[async_trait]
impl StatelessHandler for ProcessSandboxHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        // Call the transport directly and map its `SandboxError` to the
        // engine's `ActionError` taxonomy here. The transport crate does
        // not know about `ActionError`; the round-trip is raced against
        // the action's cancellation token (a pre-cancelled token resolves
        // on the first poll, so no separate pre-flight check is needed).
        self.sandbox
            .invoke_with_cancel(self.local_key.as_str(), input, context.cancellation())
            .await
            .map(ActionResult::success)
            .map_err(sandbox_error_to_action_error)
    }
}
