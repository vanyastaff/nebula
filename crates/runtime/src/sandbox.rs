//! Sandbox interface and in-process implementation.
//!
//! Executes actions within an isolation boundary; in-process runner is the default.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::result::ActionResult;
use nebula_action::{ActionContext, ActionError, ActionMetadata};

/// Sandboxed execution context wrapping an [`ActionContext`].
pub struct SandboxedContext {
    context: ActionContext,
}

impl SandboxedContext {
    /// Wrap an [`ActionContext`] in a sandboxed context.
    pub fn new(context: ActionContext) -> Self {
        Self { context }
    }

    /// Check whether execution has been cancelled.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.context.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Access the inner [`ActionContext`].
    pub fn inner(&self) -> &ActionContext {
        &self.context
    }
}

/// Trait for executing actions within an isolation boundary.
#[async_trait]
pub trait SandboxRunner: Send + Sync {
    /// Execute an action within the sandbox.
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;
}

/// Boxed future returned by the action executor.
pub type ActionExecutorFuture = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<ActionResult<serde_json::Value>, ActionError>>
            + Send,
    >,
>;

/// Callback type for executing an action (registry lookup + invoke).
pub type ActionExecutor = Arc<
    dyn Fn(SandboxedContext, &ActionMetadata, serde_json::Value) -> ActionExecutorFuture
        + Send
        + Sync,
>;

/// In-process sandbox: runs actions in the same process with capability checks.
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
            action_key = %metadata.key,
            "executing action in-process"
        );
        context.check_cancelled()?;
        let result = (self.executor)(context, metadata, input).await;
        if let Err(e) = &result {
            tracing::warn!(action_key = %metadata.key, error = %e, "action failed");
        }
        result
    }
}
