//! Execution sub-traits for actions.
//!
//! [`StatelessAction`] is the core trait for pure, stateless node execution.
//! StatefulAction, TriggerAction, and ResourceAction are planned (see API.md).
//!
//! ## Cancellation
//!
//! **Runtime responsibility:** The engine/runtime must enforce cancellation by racing
//! `action.execute(...)` against the context's cancellation token (e.g. `tokio::select!`
//! with `cancellation.cancelled()`). When cancellation wins, the runtime returns
//! `ActionError::Cancelled` to the caller. Action authors do not need to check
//! cancellation in every action — no boilerplate.

use crate::action::Action;
use crate::context::Context;
use crate::error::ActionError;
use crate::result::ActionResult;

/// Stateless action: pure function from input to result.
///
/// No state is kept between executions. The engine may run multiple
/// instances in parallel. Use StatefulAction for iterative or stateful behavior (when available).
///
/// # Cancellation
///
/// Cancellation is handled by the runtime (e.g. `tokio::select!` between execute and
/// `ctx.cancellation().cancelled()`). Implementations do not need to check
/// cancellation unless they want cooperative checks at specific points.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::{Action, StatelessAction, ActionContext, ActionResult, ActionError};
///
/// struct MyAction { meta: ActionMetadata }
/// impl Action for MyAction { ... }
///
/// impl StatelessAction for MyAction {
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///
///     async fn execute(&self, input: Self::Input, _ctx: &impl Context)
///         -> Result<ActionResult<Self::Output>, ActionError>
///     {
///         Ok(ActionResult::success(input))
///     }
/// }
/// ```
pub trait StatelessAction: Action {
    /// Input type for this action.
    type Input: Send + Sync;
    /// Output type produced on success (wrapped in [`ActionResult`]).
    type Output: Send + Sync;

    /// Execute the action with the given input and context.
    ///
    /// Returns [`ActionResult`] for flow control (Success, Skip, Branch, Wait, etc.)
    /// or [`ActionError`] for retryable/fatal failures.
    ///
    /// The returned future must be `Send` so the runtime can run it in `tokio::select!`
    /// with cancellation (no per-action cancellation boilerplate).
    fn execute(
        &self,
        input: Self::Input,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}
