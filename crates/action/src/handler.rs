//! Type-erased internal handler for action execution.
//!
//! This module is `#[doc(hidden)]` -- action authors should implement
//! [`ProcessAction`](crate::ProcessAction) or other typed traits instead.

use async_trait::async_trait;
use nebula_parameter::collection::ParameterCollection;

use crate::context::ActionContext;
use crate::error::ActionError;
use crate::metadata::{ActionMetadata, ActionType};
use crate::result::ActionResult;

/// Type-erased action handler used internally by the runtime.
///
/// Bridges typed action traits (like [`ProcessAction`](crate::ProcessAction))
/// to the runtime's JSON-based execution pipeline. Action authors should
/// **never** implement this directly -- use the typed traits and register
/// via `NodeComponents` instead.
#[async_trait]
pub trait InternalHandler: Send + Sync + 'static {
    /// Execute the action with JSON input, returning a JSON result.
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;

    /// The action's static metadata.
    fn metadata(&self) -> &ActionMetadata;

    /// The discriminant for this action type.
    ///
    /// Defaults to the `action_type` field from [`ActionMetadata`].
    fn action_type(&self) -> ActionType {
        self.metadata().action_type
    }

    /// User-facing parameter definitions, if any.
    fn parameters(&self) -> Option<&ParameterCollection>;
}
