//! Dynamic handler trait for runtime execution.
//!
//! The runtime looks up actions by key and calls `execute` with JSON input and
//! [`ActionContext`]. This trait is the contract between registry and runtime.

use async_trait::async_trait;

use crate::context::ActionContext;
use crate::error::ActionError;
use crate::metadata::ActionMetadata;
use crate::result::ActionResult;

/// Handler trait for action execution; runtime looks up by key and calls
/// `execute` with [`ActionContext`].
#[async_trait]
pub trait InternalHandler: Send + Sync {
    /// Get action metadata.
    fn metadata(&self) -> &ActionMetadata;
    /// Execute the action with the given input and execution context.
    async fn execute(
        &self,
        input: serde_json::Value,
        context: &ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;
}
