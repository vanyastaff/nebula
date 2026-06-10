//! Sandbox runner abstraction — the engine-side dispatch boundary between
//! the action dispatcher and the isolation transport.
//!
//! The dispatcher owns the runner trait: it decides, per `IsolationLevel`,
//! how an action is executed. Today the sole runner is [`InProcessSandbox`];
//! isolation is a future additive concern that does not require a second
//! runner implementation.
//!
//! ## Key types
//!
//! - [`SandboxRunner`] — execute an action within an isolation boundary.
//! - [`InProcessSandbox`] — the sole runner; trusted in-process dispatch.
//! - [`SandboxedContext`] — cooperative cancellation check for the runner.
//! - [`ActionExecutor`] — registry-lookup-and-invoke callback.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{ActionContext, ActionError, ActionMetadata, result::ActionResult};
use tokio_util::sync::CancellationToken;

/// Sandboxed execution context wrapping an [`ActionContext`].
///
/// Provides a cooperative cancellation check before action execution.
pub struct SandboxedContext {
    cancellation: CancellationToken,
}

impl SandboxedContext {
    /// Build sandbox metadata from an action context.
    pub fn new(context: &dyn ActionContext) -> Self {
        Self {
            cancellation: context.cancellation().clone(),
        }
    }

    /// Check whether execution has been cancelled.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Borrow the cancellation token for long-running dispatch paths that
    /// need to `select!` against it.
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

/// Trait for executing actions within an isolation boundary.
///
/// [`InProcessSandbox`] is the sole implementation today. The trait exists
/// as the dispatch boundary so additional isolation strategies can be added
/// additively without touching call sites.
///
/// WASM is an explicit non-goal — see `docs/PRODUCT_CANON.md`.
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
    Box<dyn Future<Output = Result<ActionResult<serde_json::Value>, ActionError>> + Send>,
>;

/// Callback type for executing an action (registry lookup + invoke).
pub type ActionExecutor = Arc<
    dyn Fn(SandboxedContext, &ActionMetadata, serde_json::Value) -> ActionExecutorFuture
        + Send
        + Sync,
>;

/// In-process sandbox: runs actions in the same process (cooperative
/// cancellation check only — no isolation).
///
/// The sole [`SandboxRunner`] implementation. Suitable for all registered
/// actions today; additional isolation strategies can be introduced
/// additively as new implementations of [`SandboxRunner`].
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
