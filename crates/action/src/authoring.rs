//! Ergonomic authoring helpers for common action patterns.
//!
//! This module provides low-boilerplate adapters for action authors.

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::context::ActionContext;
use crate::dependency::ActionDependencies;
use crate::{Action, ActionError, ActionMetadata, ActionResult, Context, StatelessAction};

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
            Some(base) => {
                // Use capabilities from the base context, identity from runtime.
                ActionContext::new(
                    ctx.execution_id(),
                    ctx.node_id(),
                    ctx.workflow_id(),
                    ctx.cancellation().clone(),
                )
                .with_resources(Arc::clone(&base.resources))
                .with_credentials(Arc::clone(&base.credentials))
                .with_logger(Arc::clone(&base.logger))
            }
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
    use crate::ActionContext;
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
                // Verify context identity is accessible
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
