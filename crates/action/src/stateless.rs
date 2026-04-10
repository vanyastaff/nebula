//! Core [`StatelessAction`] trait and function-backed DX adapters.
//!
//! Stateless actions are pure functions from input to result — no state is
//! kept between executions, and the engine may run multiple instances in
//! parallel. For iterative execution with persistent state, use
//! [`StatefulAction`](crate::stateful::StatefulAction).
//!
//! ## Cancellation
//!
//! Cancellation is handled by the runtime (e.g. `tokio::select!` between
//! `execute` and `ctx.cancellation().cancelled()`). Implementations do not
//! need to check cancellation unless they want cooperative checks at specific
//! points.
//!
//! ## DX adapters
//!
//! - [`FnStatelessAction`] / [`stateless_fn`] — zero-boilerplate adapter for a
//!   plain `async fn(Input) -> Result<Output, ActionError>`.
//! - [`FnStatelessCtxAction`] / [`stateless_ctx_fn`] — context-aware variant
//!   for closures that need credentials, resources, or the logger.

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::action::Action;
use crate::context::{ActionContext, Context};
use crate::dependency::ActionDependencies;
use crate::error::ActionError;
use crate::metadata::ActionMetadata;
use crate::result::ActionResult;

/// Stateless action: pure function from input to result.
///
/// No state is kept between executions. The engine may run multiple
/// instances in parallel. Use [`StatefulAction`](crate::stateful::StatefulAction)
/// for iterative or stateful behavior.
///
/// # Cancellation
///
/// Cancellation is handled by the runtime (e.g. `tokio::select!` between
/// `execute` and `ctx.cancellation().cancelled()`). Implementations do not
/// need to check cancellation unless they want cooperative checks at specific
/// points.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::{Action, StatelessAction, ActionContext, ActionResult, ActionError};
///
/// struct MyAction { meta: ActionMetadata }
/// impl Action for MyAction { /* ... */ }
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
    /// The returned future must be `Send` so the runtime can run it in
    /// `tokio::select!` with cancellation (no per-action cancellation boilerplate).
    fn execute(
        &self,
        input: Self::Input,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}

// ── FnStatelessAction ───────────────────────────────────────────────────────

/// Stateless action adapter backed by an async function/closure.
///
/// This removes boilerplate for the common "pure stateless transform" case.
pub struct FnStatelessAction<F, Input, Output> {
    metadata: ActionMetadata,
    func: F,
    _marker: PhantomData<fn(Input) -> Output>,
}

impl<F, Input, Output> FnStatelessAction<F, Input, Output> {
    /// Create a new function-backed stateless action.
    #[must_use]
    pub fn new(metadata: ActionMetadata, func: F) -> Self {
        Self {
            metadata,
            func,
            _marker: PhantomData,
        }
    }
}

impl<F, Input, Output> ActionDependencies for FnStatelessAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
}

impl<F, Input, Output> Action for FnStatelessAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl<F, Fut, Input, Output> StatelessAction for FnStatelessAction<F, Input, Output>
where
    F: Fn(Input) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Output, ActionError>> + Send + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    type Input = Input;
    type Output = Output;

    fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send {
        let fut = (self.func)(input);
        async move { fut.await.map(ActionResult::success) }
    }
}

/// Build a function-backed stateless action.
#[must_use]
pub fn stateless_fn<F, Input, Output>(
    metadata: ActionMetadata,
    func: F,
) -> FnStatelessAction<F, Input, Output> {
    FnStatelessAction::new(metadata, func)
}

// ── FnStatelessCtxAction ────────────────────────────────────────────────────

/// Stateless action adapter backed by a context-aware async function/closure.
///
/// Unlike [`FnStatelessAction`], the closure receives both the input and a
/// cloned [`ActionContext`], allowing credential/resource/logger access.
///
/// The closure signature is `Fn(Input, ActionContext) -> Future<...>`.
/// The context is cloned before each call (cheap — all capabilities are
/// behind `Arc`).
///
/// **Important:** Without [`with_context`](FnStatelessCtxAction::with_context),
/// `execute` builds a minimal context from the [`Context`] trait methods
/// (noop capabilities). Call `with_context` to inject a base
/// [`ActionContext`] whose credentials, resources, and logger are cloned
/// into each invocation.
pub struct FnStatelessCtxAction<F, Input, Output> {
    metadata: ActionMetadata,
    func: F,
    /// Base context to clone from. When `None`, a minimal context is
    /// constructed from the `Context` trait methods (noop capabilities).
    base_ctx: Option<ActionContext>,
    _marker: PhantomData<fn(Input) -> Output>,
}

impl<F, Input, Output> FnStatelessCtxAction<F, Input, Output> {
    /// Create a new context-aware function-backed stateless action.
    #[must_use]
    pub fn new(metadata: ActionMetadata, func: F) -> Self {
        Self {
            metadata,
            func,
            base_ctx: None,
            _marker: PhantomData,
        }
    }

    /// Provide a base [`ActionContext`] whose capabilities (credentials,
    /// resources, logger) will be cloned into each invocation's context.
    ///
    /// The execution identity and cancellation token are still taken from
    /// the runtime-provided context.
    #[must_use]
    pub fn with_context(mut self, ctx: ActionContext) -> Self {
        self.base_ctx = Some(ctx);
        self
    }
}

impl<F, Input, Output> ActionDependencies for FnStatelessCtxAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
}

impl<F, Input, Output> Action for FnStatelessCtxAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl<F, Fut, Input, Output> StatelessAction for FnStatelessCtxAction<F, Input, Output>
where
    F: Fn(Input, ActionContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Output, ActionError>> + Send + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    type Input = Input;
    type Output = Output;

    fn execute(
        &self,
        input: Self::Input,
        ctx: &impl Context,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send {
        let action_ctx = match &self.base_ctx {
            Some(base) => ActionContext::new(
                ctx.execution_id(),
                ctx.node_id(),
                ctx.workflow_id(),
                ctx.cancellation().clone(),
            )
            .with_resources(Arc::clone(&base.resources))
            .with_credentials(Arc::clone(&base.credentials))
            .with_logger(Arc::clone(&base.logger)),
            None => ActionContext::new(
                ctx.execution_id(),
                ctx.node_id(),
                ctx.workflow_id(),
                ctx.cancellation().clone(),
            ),
        };
        let fut = (self.func)(input, action_ctx);
        async move { fut.await.map(ActionResult::success) }
    }
}

/// Build a context-aware stateless action from a function that receives
/// input AND an owned [`ActionContext`].
///
/// The closure receives a fresh [`ActionContext`] on each call (cheap
/// clone — capabilities are behind `Arc`). Use this when the closure
/// needs access to credentials, resources, or the logger. For closures
/// that only transform data, prefer [`stateless_fn`].
///
/// # Examples
///
/// ```rust,ignore
/// let action = stateless_ctx_fn(
///     ActionMetadata::new(action_key!("example.ctx"), "Ctx", "Context-aware"),
///     |input: serde_json::Value, ctx: ActionContext| async move {
///         let _id = ctx.execution_id();
///         Ok(input)
///     },
/// );
/// ```
#[must_use]
pub fn stateless_ctx_fn<F, Input, Output>(
    metadata: ActionMetadata,
    func: F,
) -> FnStatelessCtxAction<F, Input, Output> {
    FnStatelessCtxAction::new(metadata, func)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn fn_stateless_action_executes_with_low_boilerplate() {
        let action = stateless_fn::<_, serde_json::Value, serde_json::Value>(
            ActionMetadata::new(
                nebula_core::action_key!("example.fn"),
                "Fn",
                "Function-backed action",
            ),
            |input| async move { Ok(input) },
        );

        let ctx = ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            CancellationToken::new(),
        );

        let result = action
            .execute(serde_json::json!({"hello":"world"}), &ctx)
            .await
            .unwrap();
        match result {
            ActionResult::Success { output } => {
                assert_eq!(
                    output.as_value(),
                    Some(&serde_json::json!({"hello":"world"}))
                );
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fn_stateless_ctx_action_receives_context() {
        let action = stateless_ctx_fn::<_, serde_json::Value, serde_json::Value>(
            ActionMetadata::new(
                nebula_core::action_key!("example.ctx_fn"),
                "CtxFn",
                "Context-aware function action",
            ),
            |input, ctx: ActionContext| async move {
                let _eid = ctx.execution_id;
                Ok(input)
            },
        );

        let ctx = ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            CancellationToken::new(),
        );

        let result = action
            .execute(serde_json::json!({"ctx":"aware"}), &ctx)
            .await
            .unwrap();
        match result {
            ActionResult::Success { output } => {
                assert_eq!(output.as_value(), Some(&serde_json::json!({"ctx":"aware"})));
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }
}
