//! In-process sandbox: runs actions directly in the host process.
//!
//! This is the default sandbox for trusted, built-in actions. No isolation
//! beyond capability checks on the [`SandboxedContext`].

use async_trait::async_trait;
use nebula_action::{ActionError, ActionMetadata, result::ActionResult};

use crate::{
    SandboxRunner,
    runner::{ActionExecutor, SandboxedContext},
};

/// In-process sandbox: runs actions in the same process with capability checks.
///
/// Suitable for first-party (built-in) actions that are trusted code.
/// Community/third-party plugins should use `WasmSandbox` instead.
pub struct InProcessSandbox {
    executor: ActionExecutor,
}

impl InProcessSandbox {
    /// Create a new in-process sandbox with the given action executor.
    pub fn new(executor: ActionExecutor) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl SandboxRunner for InProcessSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        tracing::debug!(
            action_key = %metadata.base.key,
            "executing action in-process"
        );
        context.check_cancelled()?;
        let result = (self.executor)(context, metadata, input).await;
        if let Err(e) = &result {
            tracing::warn!(action_key = %metadata.base.key, error = %e, "action failed");
        }
        result
    }
}
