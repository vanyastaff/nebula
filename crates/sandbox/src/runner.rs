//! Sandbox runner trait and supporting types.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{ActionContext, ActionError, ActionMetadata, result::ActionResult};
use tokio_util::sync::CancellationToken;

/// Sandboxed execution context wrapping an [`ActionContext`].
///
/// Provides capability checks (e.g., cancellation) before action execution.
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
    /// need to `select!` against it (e.g. the process-sandbox plugin
    /// round-trip).
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

/// Trait for executing actions within an isolation boundary.
///
/// Implementations provide different isolation levels:
/// - [`InProcessSandbox`](crate::InProcessSandbox) — trusted, in-process (built-in actions)
/// - [`ProcessSandbox`](crate::ProcessSandbox) — child-process dispatch over JSON envelope with
///   `PluginCapabilities` allowlists and optional OS-level hardening in `os_sandbox` (community
///   plugins)
///
/// WASM is an explicit non-goal — see `docs/PRODUCT_CANON.md` §12.6.
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
