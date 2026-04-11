//! ProcessSandboxHandler — bridges ProcessSandbox into ActionRegistry.
//!
//! Implements `StatelessHandler` so the engine can call community plugin
//! actions through the process sandbox transparently.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessHandler};

use crate::{SandboxRunner, process::ProcessSandbox, runner::SandboxedContext};

/// Wraps a [`ProcessSandbox`] as a [`StatelessHandler`].
///
/// Each `ProcessSandboxHandler` represents one action from a community plugin.
/// When the engine calls `execute()`, it spawns the plugin binary with the
/// action key and JSON input via stdin/stdout.
pub struct ProcessSandboxHandler {
    sandbox: Arc<ProcessSandbox>,
    metadata: ActionMetadata,
}

impl ProcessSandboxHandler {
    /// Create a new handler for an action backed by a process sandbox.
    pub fn new(sandbox: Arc<ProcessSandbox>, metadata: ActionMetadata) -> Self {
        Self { sandbox, metadata }
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
        context: &ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let sandboxed = SandboxedContext::new(context.clone());
        self.sandbox.execute(sandboxed, &self.metadata, input).await
    }
}
