//! Sandbox runner trait and supporting types.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{ActionContext, ActionError, ActionMetadata, result::ActionResult};

/// Sandboxed execution context wrapping an [`ActionContext`].
///
/// Provides capability checks (e.g., cancellation) before action execution.
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
///
/// Implementations provide different isolation levels:
/// - [`InProcessSandbox`](crate::InProcessSandbox) — trusted, in-process (built-in actions)
/// - Future: `WasmSandbox` — sandboxed WASM execution (community plugins)
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
