//! Sandbox runner port.
//!
//! Defines the interface for executing actions within an isolation boundary.
//! This trait was extracted from `nebula-action` so that the engine depends
//! on the port, not on a concrete sandbox implementation.

use async_trait::async_trait;
use nebula_action::result::ActionResult;
use nebula_action::{ActionContext, ActionError, ActionMetadata};

/// Sandboxed execution context wrapping an [`ActionContext`](nebula_action::ActionContext).
///
/// Holds the action context so capability checks and cancellation can be
/// enforced at the sandbox boundary.
pub struct SandboxedContext {
    context: ActionContext,
}

impl SandboxedContext {
    /// Wrap an [`ActionContext`](nebula_action::ActionContext) in a sandboxed context.
    pub fn new(context: ActionContext) -> Self {
        Self { context }
    }

    /// Check whether execution has been cancelled.
    ///
    /// Returns `Err(ActionError::Cancelled)` if the cancellation token
    /// has been triggered.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.context.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Access the inner [`ActionContext`](nebula_action::ActionContext).
    pub fn inner(&self) -> &ActionContext {
        &self.context
    }
}

/// Port trait for executing actions within an isolation boundary.
///
/// Implemented by drivers:
/// - `sandbox-inprocess`: runs in the same process with capability checks
/// - `sandbox-wasm`: runs in a WASM sandbox (full isolation)
///
/// The engine calls this instead of invoking `Action::execute` directly.
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
