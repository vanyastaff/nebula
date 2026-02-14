use async_trait::async_trait;

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;
use crate::result::ActionResult;

/// Stateless, single-execution action — the most common type.
///
/// Given input, produces output. No state preserved between executions.
/// Covers ~80% of workflow nodes: HTTP requests, data transforms, filters,
/// format conversions, API integrations.
///
/// # Type Parameters
///
/// - `Input`: data received from upstream nodes (commonly `serde_json::Value`).
/// - `Output`: data passed to downstream nodes (commonly `serde_json::Value`).
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::*;
///
/// struct JsonTransform {
///     meta: ActionMetadata,
/// }
///
/// #[async_trait]
/// impl ProcessAction for JsonTransform {
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///
///     async fn execute(
///         &self,
///         input: Self::Input,
///         ctx: &ActionContext,
///     ) -> Result<ActionResult<Self::Output>, ActionError> {
///         ctx.check_cancelled()?;
///         // Transform the input
///         let output = serde_json::json!({
///             "transformed": true,
///             "data": input,
///         });
///         Ok(ActionResult::success(output))
///     }
/// }
/// ```
#[async_trait]
pub trait ProcessAction: Action {
    /// Input data type received from upstream nodes.
    type Input: Send + Sync + 'static;
    /// Output data type passed to downstream nodes.
    type Output: Send + Sync + 'static;

    /// Execute the action with the given input.
    ///
    /// The engine calls this after input validation passes.
    /// Implementations should call `ctx.check_cancelled()` periodically
    /// in long-running operations.
    async fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;

    /// Validate input before execution (optional).
    ///
    /// Called by the engine before `execute`. Return `Err(ActionError::Validation(...))`
    /// to reject invalid input without executing.
    ///
    /// Default implementation accepts all input.
    async fn validate_input(&self, _input: &Self::Input) -> Result<(), ActionError> {
        Ok(())
    }
}
