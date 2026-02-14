//! Sandbox runner port.
//!
//! Defines the interface for executing actions within an isolation boundary.
//! This trait was extracted from `nebula-action` so that the engine depends
//! on the port, not on a concrete sandbox implementation.

use async_trait::async_trait;
use nebula_action::result::ActionResult;
use nebula_action::{ActionError, ActionMetadata, SandboxedContext};

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
