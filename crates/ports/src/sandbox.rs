//! Sandbox runner port.
//!
//! Defines the interface for executing actions within an isolation boundary.
//! This trait was extracted from `nebula-action` so that the engine depends
//! on the port, not on a concrete sandbox implementation.

use async_trait::async_trait;
use nebula_action::result::ActionResult;
use nebula_action::{ActionError, ActionMetadata};
// TODO: SandboxedContext is currently unavailable
// use nebula_action::SandboxedContext;

/// Sandboxed execution context wrapping a [`NodeContext`](nebula_action::NodeContext).
///
/// Holds the node context so capability checks and cancellation can be
/// enforced at the sandbox boundary.
pub struct SandboxedContext {
    context: nebula_action::NodeContext,
}

impl SandboxedContext {
    /// Wrap a [`NodeContext`](nebula_action::NodeContext) in a sandboxed context.
    pub fn new(context: nebula_action::NodeContext) -> Self {
        Self { context }
    }

    /// Check whether execution has been cancelled.
    ///
    /// Returns `Err(ActionError::Cancelled)` if the cancellation token
    /// has been triggered.
    pub fn check_cancelled(&self) -> Result<(), nebula_action::ActionError> {
        if self.context.cancellation.is_cancelled() {
            Err(nebula_action::ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Access the inner [`NodeContext`](nebula_action::NodeContext).
    pub fn inner(&self) -> &nebula_action::NodeContext {
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
