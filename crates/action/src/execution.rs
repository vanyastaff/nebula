//! Execution sub-traits for actions.
//!
//! - [`StatelessAction`] — pure function, no state between calls.
//! - [`StatefulAction`] — iterative execution with persistent state (`Continue`/`Break`).
//! - [`TriggerAction`] — workflow starter: `start`/`stop`, lives outside execution graph.
//! - [`ResourceAction`] — graph-level DI: `configure` runs before downstream, `cleanup` on scope drop.
//!
//! ## Cancellation
//!
//! **Runtime responsibility:** The engine/runtime must enforce cancellation by racing
//! `action.execute(...)` against the context's cancellation token (e.g. `tokio::select!`
//! with `cancellation.cancelled()`). When cancellation wins, the runtime returns
//! `ActionError::Cancelled` to the caller. Action authors do not need to check
//! cancellation in every action — no boilerplate.

use crate::action::Action;
use crate::context::{Context, TriggerContext};
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

/// Stateful action: iterative execution with persistent state.
///
/// The engine calls `execute` repeatedly. Return [`ActionResult::Continue`] to
/// request another iteration (state is saved); return [`ActionResult::Break`]
/// when done. Use for pagination, long-running loops, or multi-step processing.
///
/// State must be serializable (`Serialize + DeserializeOwned`) so the engine can
/// checkpoint it between iterations, and `Clone` so it can snapshot before
/// executing (rollback on failure).
///
/// Cancellation is enforced by the runtime (same as [`StatelessAction`]).
pub trait StatefulAction: Action {
    /// Input type for each iteration.
    type Input: Send + Sync;
    /// Output type (wrapped in [`ActionResult`]); `Continue` and `Break` carry output.
    type Output: Send + Sync;
    /// Persistent state type (saved between iterations by the engine).
    ///
    /// Must be serializable for engine checkpointing and cloneable for
    /// pre-execution snapshots.
    type State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync;

    /// Create initial state for the first iteration.
    ///
    /// Called once when the engine starts executing this action. Subsequent
    /// iterations receive the state mutated by the previous `execute` call.
    fn init_state(&self) -> Self::State;

    /// Execute one iteration with the given input, mutable state, and context.
    ///
    /// Return `Continue { output, progress, delay }` for another iteration,
    /// or `Break { output, reason }` when finished.
    fn execute(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}

/// Trigger action: workflow starter, lives outside the execution graph.
///
/// The runtime calls `start` to begin listening (e.g. webhook, poll); `stop`
/// to tear down. Triggers emit new workflow executions; they do not run inside one.
///
/// Uses [`TriggerContext`] (workflow_id, trigger_id, cancellation), not [`Context`].
pub trait TriggerAction: Action {
    /// Start the trigger (register listener, schedule poll, etc.).
    fn start(&self, ctx: &TriggerContext) -> impl Future<Output = Result<(), ActionError>> + Send;

    /// Stop the trigger (unregister, cancel schedule).
    fn stop(&self, ctx: &TriggerContext) -> impl Future<Output = Result<(), ActionError>> + Send;
}

/// Resource action: graph-level dependency injection.
///
/// The engine runs `configure` before downstream nodes; the resulting config
/// (or instance) is scoped to the branch. When the scope ends, the engine
/// calls `cleanup`. Use for connection pools, caches, or other resources
/// visible only to the downstream subtree (unlike `ctx.resource()` from the
/// global registry).
pub trait ResourceAction: Action {
    /// Configuration or instance type produced by `configure` and passed to downstream.
    type Config: Send + Sync;
    /// Instance type to clean up (often the same as `Config`, e.g. a pool handle).
    type Instance: Send + Sync;

    /// Build the resource for this scope; engine runs this before downstream nodes.
    fn configure(
        &self,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<Self::Config, ActionError>> + Send;

    /// Clean up the instance when the scope ends (e.g. drop pool, close connections).
    fn cleanup(
        &self,
        instance: Self::Instance,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}
