//! Sandbox runner abstraction — the engine-side boundary between the
//! action dispatcher and the isolation transport.
//!
//! The dispatcher owns the runner trait: it is the consumer that decides,
//! per `IsolationLevel`, whether an action
//! runs in-process (trusted built-ins) or through the out-of-process
//! transport (community plugins). The transport crate (`nebula-sandbox`)
//! stays free of `nebula_action`: the `SandboxError` -> `ActionError` and
//! `Value` -> `ActionResult` mapping lives here, in the adapter that bridges
//! `ProcessSandbox` to [`SandboxRunner`].
//!
//! ## Key types
//!
//! - [`SandboxRunner`] — execute an action within an isolation boundary.
//! - [`InProcessSandbox`] — trusted in-process dispatch; no isolation.
//! - [`SandboxedContext`] — cooperative cancellation check for the runner.
//! - [`ActionExecutor`] — registry-lookup-and-invoke callback.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{ActionContext, ActionError, ActionMetadata, result::ActionResult};
use nebula_plugin::sandbox_error_to_action_error;
use nebula_sandbox::ProcessSandbox;
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
    /// need to `select!` against it (e.g. the process-sandbox plugin
    /// round-trip).
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

/// Trait for executing actions within an isolation boundary.
///
/// Implementations provide different isolation levels:
/// - [`InProcessSandbox`] — trusted, in-process (built-in actions)
/// - `ProcessSandbox` — child-process dispatch over a JSON
///   envelope with Linux OS-level hardening (community plugins)
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

/// In-process sandbox: runs actions in the same process (cooperative
/// cancellation check only — no isolation).
///
/// Suitable for first-party (built-in) actions that are trusted code.
/// Untrusted/community plugins run out-of-process via
/// `ProcessSandbox`.
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

/// Adapter: drive an out-of-process [`ProcessSandbox`] as a
/// [`SandboxRunner`].
///
/// The plugin output `Value` is wrapped in an `ActionResult`; the
/// transport's `SandboxError` is classified into the engine's
/// `ActionError` taxonomy via the single shared
/// `sandbox_error_to_action_error`
/// seam. The transport crate owns neither — keeping `nebula-sandbox` a
/// Business-dependency-free leaf.
#[async_trait]
impl SandboxRunner for ProcessSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        context.check_cancelled()?;

        let action_key = metadata.base.key.as_str();

        tracing::debug!(
            action_key = %action_key,
            plugin = %self.binary().display(),
            "executing action in process sandbox"
        );

        self.invoke_with_cancel(action_key, input, context.cancellation())
            .await
            .map(ActionResult::success)
            .map_err(sandbox_error_to_action_error)
    }
}
